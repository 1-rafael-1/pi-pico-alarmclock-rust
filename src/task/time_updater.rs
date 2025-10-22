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
include!(concat!(env!("OUT_DIR"), "/time_api_config.rs"));

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
    rtc::Rtc,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
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
type RtcType = Mutex<CriticalSectionRawMutex, Option<Rtc<'static, peripherals::RTC>>>;
/// The RTC mutex, which is used to access the RTC from multiple tasks. There was no apparent place to put this anywhere else, so it is here.
pub static RTC_MUTEX: RtcType = Mutex::new(None);

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
            time_api_url: TIME_SERVER_URL,
            refresh_after_secs: 21_600, // 6 hours
            retry_after_secs: 30,
            timeout_duration: Duration::from_secs(10),
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

/// RTC management task that stores the RTC in a static mutex for access by other tasks.
#[embassy_executor::task]
async fn rtc_task(rtc: embassy_rp::rtc::Rtc<'static, embassy_rp::peripherals::RTC>) {
    // RTC management task - store RTC in static context here
    {
        *(RTC_MUTEX.lock().await) = Some(rtc);
    }

    // Keep the task alive
    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(60)).await;
    }
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

    let fw = include_bytes!("../wifi-firmware/cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../wifi-firmware/cyw43-firmware/43439A0_clm.bin");

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
    // Wait for DHCP
    let mut timeout_counter = 0;
    while !stack.is_config_up() {
        Timer::after_millis(100).await;
        timeout_counter += 1;
        if timeout_counter > 100 {
            warn!("DHCP timeout");
            return Err("DHCP timeout");
        }
    }

    // Wait for link
    timeout_counter = 0;
    while !stack.is_link_up() {
        Timer::after_millis(500).await;
        timeout_counter += 1;
        if timeout_counter > 100 {
            warn!("Link timeout");
            return Err("Link timeout");
        }
    }

    stack.wait_config_up().await;
    Ok(())
}

/// API response structure for time data.
#[derive(Deserialize)]
struct ApiResponse<'a> {
    /// ISO 8601 datetime string
    datetime: &'a str,
    /// Day of week (0-6, where 0 is Sunday)
    day_of_week: u8,
}

/// Fetch time data from the `API` using static buffers.
#[allow(clippy::significant_drop_tightening)]
async fn fetch_time_from_api(
    stack: &embassy_net::Stack<'static>,
    url: &str,
    seed: u64,
) -> Result<heapless::String<8192>, &'static str> {
    let mut buffers_guard = HTTP_BUFFERS.lock().await;
    let buffers = buffers_guard.as_mut().ok_or("HTTP buffers not available")?;

    let client_state = TcpClientState::<1, 1024, 1024>::new();
    let tcp_client = TcpClient::new(*stack, &client_state);
    let dns_client = dns::DnsSocket::new(*stack);
    let _tls_config = TlsConfig::new(
        seed,
        &mut buffers.tls_read_buffer,
        &mut buffers.tls_write_buffer,
        TlsVerify::None,
    );

    let mut http_client = HttpClient::new(&tcp_client, &dns_client);

    let mut request = http_client
        .request(Method::GET, url)
        .await
        .map_err(|_| "Failed to create HTTP request")?;

    let response = request
        .send(&mut buffers.rx_buffer)
        .await
        .map_err(|_| "Failed to send HTTP request")?;

    let response_bytes = response
        .body()
        .read_to_end()
        .await
        .map_err(|_| "Failed to read response body")?;

    let body_str = from_utf8(response_bytes).map_err(|_| "Failed to parse response as UTF-8")?;

    info!("Response body: {:?}", &body_str);

    // Copy to a heapless string to avoid lifetime issues
    heapless::String::try_from(body_str).map_err(|_| "Response too large for buffer")
}

/// Parse the time `API` response and return datetime and day of week.
fn parse_time_response(body: &str) -> Result<(&str, u8), &'static str> {
    let bytes = body.as_bytes();
    let response: ApiResponse = serde_json_core::de::from_slice::<ApiResponse>(bytes)
        .map_err(|_| "Failed to parse JSON response")?
        .0;

    info!("Datetime: {:?}", response.datetime);
    info!("Day of week: {:?}", response.day_of_week);

    Ok((response.datetime, response.day_of_week))
}

/// Update the RTC with the fetched time data.
#[allow(clippy::significant_drop_tightening)]
async fn update_rtc_with_time(datetime_str: &str, day_of_week: u8) -> Result<(), &'static str> {
    let dt = StringUtils::convert_str_to_datetime(datetime_str, day_of_week);

    {
        let mut rtc_guard = RTC_MUTEX.lock().await;
        let rtc = rtc_guard.as_mut().ok_or("RTC not initialized")?;
        rtc.set_datetime(dt).map_err(|_| "Failed to set datetime")?;
    }

    // Send event to state manager
    send_event(Event::RtcUpdated).await;
    Ok(())
}

/// Disconnect from `WiFi` and turn off `LED`.
async fn disconnect_wifi(control: &mut cyw43::Control<'static>) {
    control.leave().await;
    control.gpio_set(0, false).await;
    info!("Disconnected from wifi");
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
pub async fn time_updater(spawner: Spawner, rtc: Rtc<'static, peripherals::RTC>, wifi_peripherals: WifiPeripherals) {
    info!("time updater task started");

    // Initialize RTC task
    info!("init rtc");
    spawner.spawn(unwrap!(rtc_task(rtc)));

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
    if let Err(e) = connect_to_wifi(control, ssid, password, config.timeout_duration).await {
        disconnect_wifi(control).await;
        return Err(e);
    }

    // Wait for network to be ready
    if let Err(e) = wait_for_network_ready(stack).await {
        disconnect_wifi(control).await;
        return Err(e);
    }

    // Fetch time from API
    let body = match fetch_time_from_api(stack, config.time_api_url(), seed).await {
        Ok(b) => b,
        Err(e) => {
            disconnect_wifi(control).await;
            return Err(e);
        }
    };

    // Parse the response
    let (datetime_str, day_of_week) = match parse_time_response(&body) {
        Ok(data) => data,
        Err(e) => {
            disconnect_wifi(control).await;
            return Err(e);
        }
    };

    // Update RTC
    if let Err(e) = update_rtc_with_time(datetime_str, day_of_week).await {
        disconnect_wifi(control).await;
        return Err(e);
    }

    // Cleanup
    disconnect_wifi(control).await;
    control
        .set_power_management(cyw43::PowerManagementMode::Aggressive)
        .await;

    Ok(())
}
