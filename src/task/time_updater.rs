//! # Time Updater Task
//! This module contains the task that updates the RTC using a time API.
//! The task is responsible for connecting to a wifi network, making a request to a time API, parsing the response, and updating the RTC.
//!
//! # populate constants SSID and PASSWORD
//! make sure to have a `wifi_config.json` file in the config folder formatted as follows:
//!```json
//!  {
//!     "ssid": "some_ssid_here",
//!     "password": "some_password_here"
//! }
//! ```
//! also make sure that `build.rs` loads the `wifi_config.json` file and writes it to `wifi_secrets.rs`
//!
//! # populate constant `TIME_SERVER_URL`
//! make sure to have a `time_api_config.json` file in the config folder formatted as follows:
//! ```json
//! {
//!     "time api by zone": {
//!         "baseurl": "http://worldtimeapi.org/api",
//!         "timezone": "/timezone/Europe/Berlin"
//!     }
//! }
//! ```

include!(concat!(env!("OUT_DIR"), "/wifi_secrets.rs"));

use core::str::from_utf8;

use cyw43::JoinOptions;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use defmt::{info, unwrap, warn};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_net::{
    Config, DhcpConfig, StackResources, dns,
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_rp::{
    Peri,
    clocks::RoscRng,
    gpio::{Level, Output},
    peripherals::{self, DMA_CH0, PIO0},
    pio::Pio,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer, with_timeout};
use heapless;
use panic_probe as _;
use reqwless::{
    client::{HttpClient, TlsConfig, TlsVerify},
    request::Method,
};
use serde::Deserialize;
use serde_json_core;
use static_cell::StaticCell;

use crate::{
    Irqs,
    event::{Event, send_event},
    task::watchdog::{TaskId, report_task_failure, report_task_success},
    utility::string_utils::StringUtils,
};

/// Signal for suspending the time updater task
static TIME_UPDATER_SUSPEND_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal for resuming the time updater task
static TIME_UPDATER_RESUME_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signals the time updater to suspend
pub fn signal_time_updater_suspend() {
    TIME_UPDATER_SUSPEND_SIGNAL.signal(());
}

/// Signals the time updater to resume
pub fn signal_time_updater_resume() {
    TIME_UPDATER_RESUME_SIGNAL.signal(());
}

/// Checks if the time updater suspend signal has been signaled
fn is_time_updater_suspend_signaled() -> bool {
    TIME_UPDATER_SUSPEND_SIGNAL.signaled()
}

/// Resets the time updater suspend signal
fn reset_time_updater_suspend_signal() {
    TIME_UPDATER_SUSPEND_SIGNAL.reset();
}

/// Waits for the time updater resume signal
async fn wait_for_time_updater_resume() {
    TIME_UPDATER_RESUME_SIGNAL.wait().await;
}

/// `WiFi` peripheral resources needed for the time updater task
pub struct WifiPeripherals {
    /// Power pin for `WiFi` module
    pub pwr_pin: Peri<'static, peripherals::PIN_23>,
    /// Chip select pin for `WiFi` module
    pub cs_pin: Peri<'static, peripherals::PIN_25>,
    /// `PIO` peripheral for `WiFi` communication
    pub pio: Peri<'static, peripherals::PIO0>,
    /// Data I/O pin for `WiFi` module
    pub dio_pin: Peri<'static, peripherals::PIN_24>,
    /// Clock pin for `WiFi` module
    pub clk_pin: Peri<'static, peripherals::PIN_29>,
    /// `DMA` channel for `WiFi` communication
    pub dma_ch: Peri<'static, peripherals::DMA_CH0>,
}

/// Type alias for the RTC mutex.
/// Static cell for `CYW43` `WiFi` state.
static WIFI_STATE: StaticCell<cyw43::State> = StaticCell::new();

/// Static cell for network stack.
static NETWORK_STACK: StaticCell<embassy_net::Stack<'_>> = StaticCell::new();

/// Static cell for network stack resources.
static NETWORK_RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();

/// Static buffers for HTTP communication (protected by mutex to allow reuse).
static HTTP_BUFFERS: embassy_sync::mutex::Mutex<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    Option<HttpBuffers>,
> = embassy_sync::mutex::Mutex::new(Some(HttpBuffers::new()));

/// HTTP communication buffers.
#[allow(clippy::struct_field_names)]
struct HttpBuffers {
    /// Receive buffer for `HTTP` responses
    rx_buffer: [u8; 8192],
    /// `TLS` read buffer
    tls_read_buffer: [u8; 16640],
    /// `TLS` write buffer
    tls_write_buffer: [u8; 16640],
}

impl HttpBuffers {
    /// Create new `HTTP` buffers initialized to zero.
    #[allow(clippy::large_stack_arrays)]
    const fn new() -> Self {
        Self {
            rx_buffer: [0; 8192],
            tls_read_buffer: [0; 16640],
            tls_write_buffer: [0; 16640],
        }
    }
}

/// Configuration for the time updater task.
pub struct TimeUpdater {
    /// `WiFi` SSID
    ssid: &'static str,
    /// `WiFi` password
    password: &'static str,
    /// Time API URL
    time_api_url: &'static str,
    /// Seconds to wait before refreshing time
    refresh_after_secs: u64,
    /// Seconds to wait before retrying on error
    retry_after_secs: u64,
    /// Timeout duration for network operations
    timeout_duration: Duration,
}

impl TimeUpdater {
    /// Creates a new `TimeUpdater` instance with default configuration.
    pub const fn new() -> Self {
        Self {
            ssid: SSID,
            password: PASSWORD,
            time_api_url: "http://worldclockapi.com/api/json/cet/now",
            refresh_after_secs: 21_600, // 6 hours
            retry_after_secs: 30,
            timeout_duration: Duration::from_secs(15),
        }
    }

    /// Returns the `WiFi` credentials as a tuple of (ssid, password).
    const fn credentials(&self) -> (&str, &str) {
        (self.ssid, self.password)
    }

    /// Returns the time API URL.
    const fn time_api_url(&self) -> &str {
        self.time_api_url
    }
}

/// `WiFi` driver task that runs the `CYW43` firmware.
#[embassy_executor::task]
async fn wifi_task(runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>) -> ! {
    runner.run().await
}

/// Network stack task that handles TCP/IP networking.
#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

/// Initialize `WiFi` hardware and return the control handle and network device.
async fn setup_wifi(
    spawner: &Spawner,
    wifi_peripherals: WifiPeripherals,
) -> (cyw43::Control<'static>, cyw43::NetDriver<'static>) {
    info!("init wifi");
    let pwr = Output::new(wifi_peripherals.pwr_pin, Level::Low);
    let cs = Output::new(wifi_peripherals.cs_pin, Level::High);
    let mut pio = Pio::new(wifi_peripherals.pio, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        wifi_peripherals.dio_pin,
        wifi_peripherals.clk_pin,
        wifi_peripherals.dma_ch,
    );

    let fw = include_bytes!("../../wifi-firmware/cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../../wifi-firmware/cyw43-firmware/43439A0_clm.bin");

    let state = WIFI_STATE.init(cyw43::State::new());

    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    spawner.spawn(unwrap!(wifi_task(runner)));

    info!("init control");
    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::Aggressive)
        .await;

    (control, net_device)
}

/// Setup network stack with DHCP configuration.
fn setup_network_stack(
    spawner: &Spawner,
    net_device: cyw43::NetDriver<'static>,
    seed: u64,
) -> &'static embassy_net::Stack<'static> {
    let mut default_config = DhcpConfig::default();
    // Hostname is a valid const string, so this won't fail
    default_config.hostname = "alarmclck".try_into().ok();
    let config = Config::dhcpv4(default_config);

    let (stack, runner) = embassy_net::new(
        net_device,
        config,
        NETWORK_RESOURCES.init(StackResources::<5>::new()),
        seed,
    );
    let stack = NETWORK_STACK.init(stack);
    spawner.spawn(unwrap!(net_task(runner)));
    stack
}

/// Connect to `WiFi` network with timeout handling.
async fn connect_to_wifi(
    control: &mut cyw43::Control<'static>,
    ssid: &str,
    password: &str,
    timeout: Duration,
) -> Result<(), &'static str> {
    let join_result = with_timeout(timeout, control.join(ssid, JoinOptions::new(password.as_bytes()))).await;

    match join_result {
        Ok(Ok(())) => {
            control.gpio_set(0, true).await;
            info!("Connected to wifi");
            Ok(())
        }
        Ok(Err(_)) => {
            warn!("Error connecting to wifi");
            Err("Failed to join network")
        }
        Err(_) => {
            warn!("Timeout while trying to connect to wifi");
            Err("Connection timeout")
        }
    }
}

/// Wait for network to be ready (DHCP and link up).
async fn wait_for_network_ready(stack: &embassy_net::Stack<'static>) -> Result<(), &'static str> {
    info!("Waiting for network to be ready...");

    // Wait for link first - must be up before DHCP can work
    info!("Waiting for link to be up...");
    let mut timeout_counter = 0;
    while !stack.is_link_up() {
        Timer::after_millis(100).await;
        timeout_counter += 1;
        if timeout_counter > 100 {
            warn!("Link timeout after 10 seconds");
            return Err("Link timeout");
        }
    }
    info!("Link is up");

    // Now wait for DHCP config
    info!("Waiting for DHCP config...");
    timeout_counter = 0;
    while !stack.is_config_up() {
        Timer::after_millis(100).await;
        timeout_counter += 1;
        if timeout_counter > 300 {
            warn!("DHCP timeout after 30 seconds");
            return Err("DHCP timeout");
        }
    }
    info!("DHCP config is up");

    info!("Waiting for final config...");
    stack.wait_config_up().await;
    info!("Network is fully ready");

    // Give the network stack a moment to stabilize
    info!("Waiting 500ms for network stack to stabilize...");
    Timer::after_millis(500).await;

    Ok(())
}

/// API response structure for time data from worldclockapi.com
#[derive(Deserialize)]
struct WorldClockApiResponse<'a> {
    /// ISO 8601 datetime string (e.g., "2025-01-15T19:10+01:00")
    #[serde(rename = "currentDateTime")]
    current_date_time: &'a str,
    /// Day of week as string (e.g., "Saturday")
    #[serde(rename = "dayOfTheWeek")]
    day_of_the_week: &'a str,
}

/// API response structure for worldtimeapi.org (time.now-compatible fields).
#[derive(Deserialize)]
struct TimeNowApiResponse<'a> {
    /// ISO 8601 datetime string (e.g., "2023-10-05T14:30:00.123456+01:00")
    datetime: &'a str,
    /// Day of week as number (0 = Sunday, 6 = Saturday)
    day_of_week: u8,
}

/// Fetch time data from the `API` using static buffers.
#[allow(clippy::significant_drop_tightening)]
async fn fetch_time_from_api(
    stack: &embassy_net::Stack<'static>,
    url: &str,
    seed: u64,
    timeout_duration: Duration,
) -> Result<heapless::String<8192>, &'static str> {
    info!("Starting HTTP request to: {}", url);

    let mut buffers_guard = HTTP_BUFFERS.lock().await;
    let buffers = buffers_guard.as_mut().ok_or("HTTP buffers not available")?;
    info!("HTTP buffers acquired");

    let client_state = TcpClientState::<1, 1024, 1024>::new();
    let tcp_client = TcpClient::new(*stack, &client_state);
    let dns_client = dns::DnsSocket::new(*stack);
    info!("TCP and DNS clients created");

    let max_retries = 2;
    let body = if url.starts_with("https://") {
        let mut tls_config = TlsConfig::new(
            seed,
            &mut buffers.tls_read_buffer,
            &mut buffers.tls_write_buffer,
            TlsVerify::None,
        );
        info!("TLS config created for HTTPS");

        let mut http_client = HttpClient::new_with_tls(&tcp_client, &dns_client, tls_config);
        info!("HTTPS client created");

        let mut attempt = 0;
        let body = loop {
            info!(
                "Creating HTTP GET request... (attempt {}/{})",
                attempt + 1,
                max_retries + 1
            );
            let mut request = match http_client.request(Method::GET, url).await {
                Ok(req) => req,
                Err(e) => {
                    warn!(
                        "Failed to create HTTP request (could be DNS or connection issue): {:?}",
                        e
                    );
                    if attempt >= max_retries {
                        return Err("Failed to create HTTP request");
                    }
                    attempt += 1;
                    Timer::after(Duration::from_millis(250)).await;
                    continue;
                }
            };
            info!("HTTP request created successfully (DNS resolved, TCP connection established)");

            info!("Sending HTTP request...");
            let response = match with_timeout(timeout_duration, request.send(&mut buffers.rx_buffer)).await {
                Ok(Ok(resp)) => resp,
                Ok(Err(e)) => {
                    warn!(
                        "Failed to send HTTP request (connection reset or send failure): {:?}",
                        e
                    );
                    if attempt >= max_retries {
                        return Err("Failed to send HTTP request");
                    }
                    attempt += 1;
                    Timer::after(Duration::from_millis(250)).await;
                    continue;
                }
                Err(_) => {
                    warn!("Timed out while sending HTTP request");
                    if attempt >= max_retries {
                        return Err("Timed out while sending HTTP request");
                    }
                    attempt += 1;
                    Timer::after(Duration::from_millis(250)).await;
                    continue;
                }
            };
            info!("HTTP response received");

            info!("Reading response body...");
            let response_bytes = response.body().read_to_end().await.map_err(|e| {
                warn!("Failed to read response body: {:?}", e);
                "Failed to read response body"
            })?;
            info!("Response body read successfully, {} bytes", response_bytes.len());

            let body_str = from_utf8(response_bytes).map_err(|_| "Failed to parse response as UTF-8")?;

            info!("Response body: {:?}", &body_str);

            // Copy to a heapless string to avoid lifetime issues
            let body = heapless::String::try_from(body_str).map_err(|_| "Response too large for buffer")?;
            break body;
        };
        body
    } else {
        let mut http_client = HttpClient::new(&tcp_client, &dns_client);
        info!("HTTP client created");

        let mut attempt = 0;
        let body = loop {
            info!(
                "Creating HTTP GET request... (attempt {}/{})",
                attempt + 1,
                max_retries + 1
            );
            let mut request = match http_client.request(Method::GET, url).await {
                Ok(req) => req,
                Err(e) => {
                    warn!(
                        "Failed to create HTTP request (could be DNS or connection issue): {:?}",
                        e
                    );
                    if attempt >= max_retries {
                        return Err("Failed to create HTTP request");
                    }
                    attempt += 1;
                    Timer::after(Duration::from_millis(250)).await;
                    continue;
                }
            };
            info!("HTTP request created successfully (DNS resolved, TCP connection established)");

            info!("Sending HTTP request...");
            let response = match with_timeout(timeout_duration, request.send(&mut buffers.rx_buffer)).await {
                Ok(Ok(resp)) => resp,
                Ok(Err(e)) => {
                    warn!(
                        "Failed to send HTTP request (connection reset or send failure): {:?}",
                        e
                    );
                    if attempt >= max_retries {
                        return Err("Failed to send HTTP request");
                    }
                    attempt += 1;
                    Timer::after(Duration::from_millis(250)).await;
                    continue;
                }
                Err(_) => {
                    warn!("Timed out while sending HTTP request");
                    if attempt >= max_retries {
                        return Err("Timed out while sending HTTP request");
                    }
                    attempt += 1;
                    Timer::after(Duration::from_millis(250)).await;
                    continue;
                }
            };
            info!("HTTP response received");

            info!("Reading response body...");
            let response_bytes = response.body().read_to_end().await.map_err(|e| {
                warn!("Failed to read response body: {:?}", e);
                "Failed to read response body"
            })?;
            info!("Response body read successfully, {} bytes", response_bytes.len());

            let body_str = from_utf8(response_bytes).map_err(|_| "Failed to parse response as UTF-8")?;

            info!("Response body: {:?}", &body_str);

            // Copy to a heapless string to avoid lifetime issues
            let body = heapless::String::try_from(body_str).map_err(|_| "Response too large for buffer")?;
            break body;
        };
        body
    };

    Ok(body)
}

/// Parse worldclockapi response and return owned datetime and day of week.
fn parse_worldclockapi_response(body: &str) -> Result<(heapless::String<64>, u8), &'static str> {
    let bytes = body.as_bytes();
    let response: WorldClockApiResponse = serde_json_core::de::from_slice::<WorldClockApiResponse>(bytes)
        .map_err(|_| "Failed to parse worldclockapi JSON response")?
        .0;

    info!("WorldClockAPI datetime: {:?}", response.current_date_time);
    info!("WorldClockAPI day of week: {:?}", response.day_of_the_week);

    // Convert day of week string to number (0 = Sunday, 6 = Saturday)
    let day_num = match response.day_of_the_week {
        "Sunday" => 0,
        "Monday" => 1,
        "Tuesday" => 2,
        "Wednesday" => 3,
        "Thursday" => 4,
        "Friday" => 5,
        "Saturday" => 6,
        _ => return Err("Invalid day of week in worldclockapi response"),
    };

    let datetime = heapless::String::<64>::try_from(response.current_date_time)
        .map_err(|_| "Datetime too large in worldclockapi response")?;

    Ok((datetime, day_num))
}

/// Parse time.now response and return owned datetime and day of week.
fn parse_time_now_response(body: &str) -> Result<(heapless::String<64>, u8), &'static str> {
    let bytes = body.as_bytes();
    let response: TimeNowApiResponse = serde_json_core::de::from_slice::<TimeNowApiResponse>(bytes)
        .map_err(|_| "Failed to parse time.now JSON response")?
        .0;

    info!("time.now datetime: {:?}", response.datetime);
    info!("time.now day of week: {:?}", response.day_of_week);

    if response.day_of_week > 6 {
        return Err("Invalid day_of_week in time.now response");
    }

    let datetime =
        heapless::String::<64>::try_from(response.datetime).map_err(|_| "Datetime too large in time.now response")?;

    Ok((datetime, response.day_of_week))
}

/// Update the RTC with the fetched time data.
#[allow(clippy::significant_drop_tightening)]
async fn update_rtc_with_time(datetime_str: &str, day_of_week: u8) -> Result<(), &'static str> {
    use crate::task::rtc_manager::rtc_set_time;

    let dt = StringUtils::convert_str_to_datetime(datetime_str, day_of_week);

    rtc_set_time(dt).await.map_err(|()| "Failed to set datetime")?;

    // Send event to state manager
    send_event(Event::RtcUpdated).await;
    Ok(())
}

/// Disconnect from `WiFi` and turn off `LED`.
async fn disconnect_wifi(control: &mut cyw43::Control<'static>, stack: &embassy_net::Stack<'static>) {
    control.leave().await;
    control.gpio_set(0, false).await;
    info!("Disconnected from wifi");

    // Wait for network stack to go down
    info!("Waiting for network stack to go DOWN...");
    let mut timeout_counter = 0;
    while stack.is_link_up() || stack.is_config_up() {
        Timer::after_millis(100).await;
        timeout_counter += 1;
        if timeout_counter > 50 {
            warn!("Timeout waiting for network stack to go down");
            break;
        }
    }
    info!("Network stack is DOWN");
}

/// Handle the retry delay after an error.
async fn handle_retry_delay(retry_secs: u64, error_msg: &str) {
    warn!("{} Retrying in {:?} seconds", error_msg, retry_secs);
    Timer::after(Duration::from_secs(retry_secs)).await;
}

/// Main time updater task that periodically connects to `WiFi`, fetches time from an API,
/// and updates the `RTC`.
///
/// This task manages the entire lifecycle of `WiFi` connectivity, `HTTP` requests,
/// and `RTC` synchronization.
#[allow(clippy::large_futures)]
#[embassy_executor::task]
pub async fn time_updater(spawner: Spawner, wifi_peripherals: WifiPeripherals) {
    info!("time updater task started");

    // Initialize WiFi and network stack
    let (mut control, net_device) = setup_wifi(&spawner, wifi_peripherals).await;

    let mut rng = RoscRng;
    let seed = rng.next_u64();

    let stack = setup_network_stack(&spawner, net_device, seed);

    // Get configuration
    let time_updater = TimeUpdater::new();
    let (ssid, password) = time_updater.credentials();

    info!("starting loop");
    loop {
        // Handle suspend/resume signals
        if is_time_updater_suspend_signaled() {
            reset_time_updater_suspend_signal();
            wait_for_time_updater_resume().await;
        }

        // Attempt to update time
        if let Err(error_msg) = update_time_once(&mut control, stack, ssid, password, &time_updater, seed).await {
            // Report failure to watchdog on error path
            report_task_failure(TaskId::TimeUpdater).await;
            handle_retry_delay(time_updater.retry_after_secs, error_msg).await;
            continue;
        }

        // Successfully updated - report to watchdog before sleeping
        report_task_success(TaskId::TimeUpdater).await;

        // Wait for next refresh
        info!(
            "Waiting for {:?} seconds before reconnecting",
            time_updater.refresh_after_secs
        );
        let downtime_timer = Timer::after(Duration::from_secs(time_updater.refresh_after_secs));
        select(downtime_timer, wait_for_time_updater_resume()).await;
    }
}

/// Perform a single time update cycle.
async fn update_time_once(
    control: &mut cyw43::Control<'static>,
    stack: &embassy_net::Stack<'static>,
    ssid: &str,
    password: &str,
    config: &TimeUpdater,
    seed: u64,
) -> Result<(), &'static str> {
    // Set performance mode for connection
    control
        .set_power_management(cyw43::PowerManagementMode::Performance)
        .await;

    // Connect to WiFi
    info!(
        "Attempting to connect to WiFi - SSID: '{}', Password: '{}'",
        ssid, password
    );
    if let Err(e) = connect_to_wifi(control, ssid, password, config.timeout_duration).await {
        disconnect_wifi(control, stack).await;
        return Err(e);
    }

    // Wait for network to be ready
    if let Err(e) = wait_for_network_ready(stack).await {
        disconnect_wifi(control, stack).await;
        return Err(e);
    }

    // Fetch and parse time from the configured API.
    let time_api_url = config.time_api_url();
    info!("Time API: {}", time_api_url);

    let body = match fetch_time_from_api(stack, time_api_url, seed, config.timeout_duration).await {
        Ok(b) => b,
        Err(e) => {
            disconnect_wifi(control, stack).await;
            return Err(e);
        }
    };

    let (datetime_str, day_of_week) = match parse_worldclockapi_response(&body) {
        Ok(data) => data,
        Err(e) => {
            disconnect_wifi(control, stack).await;
            return Err(e);
        }
    };

    // Update RTC
    if let Err(e) = update_rtc_with_time(datetime_str.as_str(), day_of_week).await {
        disconnect_wifi(control, stack).await;
        return Err(e);
    }

    // Cleanup
    disconnect_wifi(control, stack).await;
    control
        .set_power_management(cyw43::PowerManagementMode::Aggressive)
        .await;

    Ok(())
}
