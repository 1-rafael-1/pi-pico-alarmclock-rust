//! # Time Updater Task
//! This module contains the task that updates the RTC using NTP.
//! The task is responsible for connecting to a wifi network, querying an NTP server, applying CET/CEST rules, and updating the RTC.
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

include!(concat!(env!("OUT_DIR"), "/wifi_secrets.rs"));

use cyw43::JoinOptions;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use defmt::{info, unwrap, warn};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_net::{
    Config, DhcpConfig, IpEndpoint, StackResources, dns,
    udp::{PacketMetadata, UdpSocket},
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
use panic_probe as _;
use static_cell::StaticCell;

use embassy_rp::rtc::{DateTime, DayOfWeek};

use crate::{
    Irqs,
    event::{Event, send_event},
    task::watchdog::{TaskId, report_task_failure, report_task_success},
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

/// Static buffers for NTP UDP communication (protected by mutex to allow reuse).
static NTP_BUFFERS: embassy_sync::mutex::Mutex<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    Option<NtpBuffers>,
> = embassy_sync::mutex::Mutex::new(Some(NtpBuffers::new()));

/// NTP UDP communication buffers.
#[allow(clippy::struct_field_names)]
struct NtpBuffers {
    /// Receive packet metadata
    rx_meta: [PacketMetadata; 4],
    /// Receive buffer
    rx_buffer: [u8; 128],
    /// Transmit packet metadata
    tx_meta: [PacketMetadata; 4],
    /// Transmit buffer
    tx_buffer: [u8; 128],
}

impl NtpBuffers {
    /// Create new NTP buffers initialized to zero.
    const fn new() -> Self {
        Self {
            rx_meta: [PacketMetadata::EMPTY; 4],
            rx_buffer: [0; 128],
            tx_meta: [PacketMetadata::EMPTY; 4],
            tx_buffer: [0; 128],
        }
    }
}

/// Configuration for the time updater task.
pub struct TimeUpdater {
    /// `WiFi` SSID
    ssid: &'static str,
    /// `WiFi` password
    password: &'static str,
    /// NTP server hostname
    ntp_server: &'static str,
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
            ntp_server: "europe.pool.ntp.org",
            refresh_after_secs: 21_600, // 6 hours
            retry_after_secs: 30,
            timeout_duration: Duration::from_secs(15),
        }
    }

    /// Returns the `WiFi` credentials as a tuple of (ssid, password).
    const fn credentials(&self) -> (&str, &str) {
        (self.ssid, self.password)
    }

    /// Returns the NTP server hostname.
    const fn ntp_server(&self) -> &str {
        self.ntp_server
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

/// NTP server port.
const NTP_PORT: u16 = 123;
/// NTP request/response size in bytes.
const NTP_PACKET_SIZE: usize = 48;
/// Seconds between NTP epoch (1900-01-01) and Unix epoch (1970-01-01).
const NTP_UNIX_EPOCH_DELTA: u64 = 2_208_988_800;

/// Fetch current UTC time from NTP server.
#[allow(clippy::significant_drop_tightening)]
async fn fetch_time_from_ntp(
    stack: &embassy_net::Stack<'static>,
    server: &str,
    timeout_duration: Duration,
) -> Result<i64, &'static str> {
    info!("Starting NTP request to: {}", server);

    let mut buffers_guard = NTP_BUFFERS.lock().await;
    let buffers = buffers_guard.as_mut().ok_or("NTP buffers not available")?;
    info!("NTP buffers acquired");

    let mut socket = UdpSocket::new(
        *stack,
        &mut buffers.rx_meta,
        &mut buffers.rx_buffer,
        &mut buffers.tx_meta,
        &mut buffers.tx_buffer,
    );

    socket.bind(0).map_err(|_| "Failed to bind UDP socket")?;

    let addrs = with_timeout(timeout_duration, stack.dns_query(server, dns::DnsQueryType::A))
        .await
        .map_err(|_| "DNS query timed out")?
        .map_err(|_| "DNS query failed")?;
    let server_ip = addrs.first().ok_or("DNS query returned no results")?;
    let endpoint = IpEndpoint::new(*server_ip, NTP_PORT);

    let mut request = [0u8; NTP_PACKET_SIZE];
    request[0] = 0x23; // LI=0, VN=4, Mode=3 (client)

    let mut response = [0u8; NTP_PACKET_SIZE];
    let max_retries = 2;

    for attempt in 0..=max_retries {
        info!("Sending NTP request... (attempt {}/{})", attempt + 1, max_retries + 1);
        let send_result = with_timeout(timeout_duration, socket.send_to(&request, endpoint)).await;
        match send_result {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                warn!("Failed to send NTP request");
                if attempt >= max_retries {
                    return Err("Failed to send NTP request");
                }
                Timer::after(Duration::from_millis(250)).await;
                continue;
            }
            Err(_) => {
                warn!("Timed out while sending NTP request");
                if attempt >= max_retries {
                    return Err("Timed out while sending NTP request");
                }
                Timer::after(Duration::from_millis(250)).await;
                continue;
            }
        }

        let recv_result = with_timeout(timeout_duration, socket.recv_from(&mut response)).await;
        match recv_result {
            Ok(Ok((n, meta))) => {
                if meta.endpoint != endpoint {
                    warn!("Ignoring NTP response from unexpected endpoint: {:?}", meta.endpoint);
                    if attempt >= max_retries {
                        return Err("Unexpected NTP response source");
                    }
                    Timer::after(Duration::from_millis(250)).await;
                    continue;
                }
                if n < NTP_PACKET_SIZE {
                    warn!("NTP response too short: {} bytes", n);
                    if attempt >= max_retries {
                        return Err("NTP response too short");
                    }
                    Timer::after(Duration::from_millis(250)).await;
                    continue;
                }

                let ntp_seconds = u64::from(u32::from_be_bytes([
                    response[40],
                    response[41],
                    response[42],
                    response[43],
                ]));

                if ntp_seconds < NTP_UNIX_EPOCH_DELTA {
                    return Err("NTP time is before Unix epoch");
                }

                let unix_seconds_u64 = ntp_seconds - NTP_UNIX_EPOCH_DELTA;
                let unix_seconds = i64::try_from(unix_seconds_u64).map_err(|_| "NTP time out of range for i64")?;
                return Ok(unix_seconds);
            }
            Ok(Err(_)) => {
                warn!("Failed to receive NTP response");
                if attempt >= max_retries {
                    return Err("Failed to receive NTP response");
                }
                Timer::after(Duration::from_millis(250)).await;
            }
            Err(_) => {
                warn!("Timed out waiting for NTP response");
                if attempt >= max_retries {
                    return Err("Timed out waiting for NTP response");
                }
                Timer::after(Duration::from_millis(250)).await;
            }
        }
    }

    Err("Failed to fetch time from NTP")
}

/// Returns whether the given year is a leap year in the Gregorian calendar.
const fn is_leap_year(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Returns the number of days in the given month for the provided year.
const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Convert a date to the number of days since Unix epoch (1970-01-01).
fn date_to_days_since_epoch(year: u16, month: u8, day: u8) -> i64 {
    let mut days: i64 = 0;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }
    for m in 1..month {
        days += i64::from(days_in_month(year, m));
    }
    days + i64::from(day - 1)
}

/// Convert a UTC date/time to Unix seconds.
fn datetime_to_unix_seconds(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> i64 {
    let days = date_to_days_since_epoch(year, month, day);
    days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second)
}

/// Map day-of-week index (0=Sunday) to `DayOfWeek`.
const fn day_of_week_from_index(idx: u8) -> DayOfWeek {
    match idx {
        0 => DayOfWeek::Sunday,
        1 => DayOfWeek::Monday,
        2 => DayOfWeek::Tuesday,
        3 => DayOfWeek::Wednesday,
        4 => DayOfWeek::Thursday,
        5 => DayOfWeek::Friday,
        _ => DayOfWeek::Saturday,
    }
}

/// Convert Unix seconds (UTC) to a `DateTime`.
fn unix_seconds_to_datetime(seconds: i64) -> DateTime {
    let mut days = seconds.div_euclid(86_400);
    let mut rem = seconds.rem_euclid(86_400);

    let hour = u8::try_from(rem / 3_600).unwrap_or(0);
    rem %= 3_600;
    let minute = u8::try_from(rem / 60).unwrap_or(0);
    let second = u8::try_from(rem % 60).unwrap_or(0);

    let mut year: u16 = 1970;
    loop {
        let year_days = if is_leap_year(year) { 366 } else { 365 };
        if days >= i64::from(year_days) {
            days -= i64::from(year_days);
            year += 1;
        } else {
            break;
        }
    }

    let mut month: u8 = 1;
    loop {
        let dim = days_in_month(year, month);
        if days >= i64::from(dim) {
            days -= i64::from(dim);
            month += 1;
        } else {
            break;
        }
    }

    let day = u8::try_from(days + 1).unwrap_or(1);
    let dow_index = u8::try_from((seconds.div_euclid(86_400) + 4).rem_euclid(7)).unwrap_or(0); // 1970-01-01 was Thursday

    DateTime {
        year,
        month,
        day,
        day_of_week: day_of_week_from_index(dow_index),
        hour,
        minute,
        second,
    }
}

/// Returns the day of month for the last Sunday in the given month.
fn last_sunday(year: u16, month: u8) -> u8 {
    let mut day = days_in_month(year, month);
    loop {
        let days_since_epoch = date_to_days_since_epoch(year, month, day);
        let dow_index = (days_since_epoch + 4).rem_euclid(7);
        if dow_index == 0 {
            return day;
        }
        day -= 1;
    }
}

/// Returns `true` if CET/CEST DST is active for the given UTC timestamp.
fn is_cet_dst(utc_seconds: i64) -> bool {
    let utc_dt = unix_seconds_to_datetime(utc_seconds);
    let year = utc_dt.year;
    let dst_start_day = last_sunday(year, 3);
    let dst_end_day = last_sunday(year, 10);

    let dst_start = datetime_to_unix_seconds(year, 3, dst_start_day, 1, 0, 0);
    let dst_end = datetime_to_unix_seconds(year, 10, dst_end_day, 1, 0, 0);

    utc_seconds >= dst_start && utc_seconds < dst_end
}

/// Convert UTC seconds to CET/CEST local time.
fn apply_cet_with_dst(utc_seconds: i64) -> DateTime {
    let offset = 3_600 + if is_cet_dst(utc_seconds) { 3_600 } else { 0 };
    unix_seconds_to_datetime(utc_seconds + offset)
}

/// Update the RTC with the fetched time data.
#[allow(clippy::significant_drop_tightening)]
async fn update_rtc_with_time(dt: DateTime) -> Result<(), &'static str> {
    use crate::task::rtc_manager::rtc_set_time;

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

/// Main time updater task that periodically connects to `WiFi`, fetches time from NTP,
/// and updates the `RTC`.
///
/// This task manages the entire lifecycle of `WiFi` connectivity, NTP queries,
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
        if let Err(error_msg) = update_time_once(&mut control, stack, ssid, password, &time_updater).await {
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

    // Fetch time from NTP and apply CET/CEST rules.
    let ntp_server = config.ntp_server();
    info!("NTP server: {}", ntp_server);

    let utc_seconds = match fetch_time_from_ntp(stack, ntp_server, config.timeout_duration).await {
        Ok(s) => s,
        Err(e) => {
            disconnect_wifi(control, stack).await;
            return Err(e);
        }
    };

    let local_dt = apply_cet_with_dst(utc_seconds);

    // Update RTC
    if let Err(e) = update_rtc_with_time(local_dt).await {
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
