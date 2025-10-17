//! # Time Updater Task
//! This module contains the task that updates the RTC using a time API.
//! The task is responsible for connecting to a wifi network, making a request to a time API, parsing the response, and updating the RTC.
//!
//! # populate constants SSID and PASSWORD
//! make sure to have a wifi_manager.json file in the config folder formatted as follows:
//!```json
//!  {
//!     "ssid": "some_ssid_here",
//!     "password": "some_password_here"
//! }
//! ```
//! also make sure that build.rs loads the wifi_manager.json file and writes it to wifi_secrets.rs
//!
//! # populate constant TIME_SERVER_URL
//! make sure to have a time_api_config.json file in the config folder formatted as follows:
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

use crate::Irqs;
use crate::task::task_messages::{
    EVENT_CHANNEL, Events, TIME_UPDATER_RESUME_SIGNAL, TIME_UPDATER_SUSPEND_SIGNAL,
};
use crate::utility::string_utils::StringUtils;
use core::str::from_utf8;
use cyw43::JoinOptions;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use defmt::*;
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
    rtc::{DateTime, Rtc},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer, with_timeout};

use reqwless::client::{HttpClient, TlsConfig, TlsVerify};
use reqwless::request::Method;
use serde::Deserialize;
use serde_json_core;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

/// WiFi peripheral resources needed for the time updater task
pub struct WifiPeripherals {
    pub pwr_pin: Peri<'static, peripherals::PIN_23>,
    pub cs_pin: Peri<'static, peripherals::PIN_25>,
    pub pio: Peri<'static, peripherals::PIO0>,
    pub dio_pin: Peri<'static, peripherals::PIN_24>,
    pub clk_pin: Peri<'static, peripherals::PIN_29>,
    pub dma_ch: Peri<'static, peripherals::DMA_CH0>,
}

/// Type alias for the RTC mutex.
type RtcType = Mutex<CriticalSectionRawMutex, Option<Rtc<'static, peripherals::RTC>>>;
/// The RTC mutex, which is used to access the RTC from multiple tasks. There was no apparent place to put this anywhere else, so it is here.
pub static RTC_MUTEX: RtcType = Mutex::new(None);

pub struct TimeUpdater {
    ssid: &'static str,
    password: &'static str,
    time_api_url: &'static str,
    refresh_after_secs: u64,
    retry_after_secs: u64,
    timeout_duration: Duration,
}

impl TimeUpdater {
    pub fn new() -> Self {
        let mut manager = TimeUpdater {
            ssid: "",
            password: "",
            time_api_url: "",
            refresh_after_secs: 21_600, // 6 hours
            retry_after_secs: 30,
            timeout_duration: Duration::from_secs(10),
        };
        manager.set_credentials();
        manager.set_time_api_url();
        manager
    }

    fn set_credentials(&mut self) {
        self.ssid = SSID;
        self.password = PASSWORD;
    }

    fn credentials(&self) -> (&str, &str) {
        (self.ssid, self.password)
    }

    fn set_time_api_url(&mut self) {
        self.time_api_url = TIME_SERVER_URL;
    }

    fn time_api_url(&self) -> &str {
        self.time_api_url
    }
}

#[embassy_executor::task]
async fn wifi_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

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

#[embassy_executor::task]
pub async fn time_updater(
    spawner: Spawner,
    rtc: Rtc<'static, peripherals::RTC>,
    wifi_peripherals: WifiPeripherals,
) {
    info!("time updater task started");

    info!("init rtc");
    // spawn RTC task with static resources
    spawner.spawn(unwrap!(rtc_task(rtc)));

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

    let time_updater = TimeUpdater::new();

    let fw = include_bytes!("../wifi-firmware/cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../wifi-firmware/cyw43-firmware/43439A0_clm.bin");

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());

    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    spawner.spawn(unwrap!(wifi_task(runner)));

    info!("init control");
    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::Aggressive)
        .await;

    let mut default_config: DhcpConfig = Default::default();
    default_config.hostname = Some("alarmclck".try_into().unwrap());
    let config = Config::dhcpv4(default_config);

    // random seed
    let mut rng = RoscRng;
    let seed = rng.next_u64();

    // Initialize the network stack
    static STACK: StaticCell<embassy_net::Stack<'_>> = StaticCell::new();
    static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        net_device,
        config,
        RESOURCES.init(StackResources::<5>::new()),
        seed,
    );
    let stack = &*STACK.init(stack);

    spawner.spawn(unwrap!(net_task(runner)));

    // Wait for link up
    info!("waiting for DHCP...");
    while !stack.is_link_up() {
        Timer::after(Duration::from_millis(500)).await;
    }

    // Wait for DHCP, not necessary when using static IP
    info!("waiting for DHCP...");
    while !stack.is_config_up() {
        Timer::after(Duration::from_millis(500)).await;
    }
    info!("DHCP is now up!");

    // get the wifi credentials
    let (ssid, password) = time_updater.credentials();

    info!("starting loop");
    '_mainloop: loop {
        if TIME_UPDATER_SUSPEND_SIGNAL.signaled() {
            TIME_UPDATER_SUSPEND_SIGNAL.reset();
            TIME_UPDATER_RESUME_SIGNAL.wait().await;
        };

        // set the power management mode to best performance for the the duration of the connection
        control
            .set_power_management(cyw43::PowerManagementMode::Performance)
            .await;

        // Join the network
        let join_result = with_timeout(
            time_updater.timeout_duration,
            control.join(&ssid, JoinOptions::new(password.as_bytes())),
        )
        .await;
        match join_result {
            Ok(Ok(_)) => {
                control.gpio_set(0, true).await; // Turn on the onboard LED
                info!("Connected to wifi");
            }
            Ok(Err(e)) => {
                warn!("Error connecting to wifi: {}", Debug2Format(&e));
                control.leave().await;
                control.gpio_set(0, false).await; // Turn off the onboard LED
                Timer::after(Duration::from_secs(time_updater.retry_after_secs)).await;
                continue;
            }
            Err(_) => {
                warn!("Timeout while trying to connect to wifi");
                control.leave().await;
                control.gpio_set(0, false).await; // Turn off the onboard LED
                Timer::after(Duration::from_secs(time_updater.retry_after_secs)).await;
                continue;
            }
        }

        // dhcp
        let mut timeout_counter = 0;
        while !stack.is_config_up() {
            Timer::after_millis(100).await;
            timeout_counter += 1;
            if timeout_counter > 100 {
                break;
            }
        }
        if !stack.is_config_up() {
            control.leave().await;
            control.gpio_set(0, false).await; // Turn off the onboard LED
            warn!(
                "Disconnected from wifi after waiting for DHCP timed out. Retrying in {:?} seconds",
                time_updater.retry_after_secs
            );
            Timer::after(Duration::from_secs(time_updater.retry_after_secs)).await;
            continue;
        }

        // link
        timeout_counter = 0;
        while !stack.is_link_up() {
            Timer::after_millis(500).await;
            timeout_counter += 1;
            if timeout_counter > 100 {
                break;
            }
        }
        if !stack.is_link_up() {
            control.leave().await;
            control.gpio_set(0, false).await; // Turn off the onboard LED
            warn!(
                "Disconnected from wifi after establishing link timed out. Retrying in {:?} seconds",
                time_updater.retry_after_secs
            );
            Timer::after(Duration::from_secs(time_updater.retry_after_secs)).await;
            continue;
        }
        stack.wait_config_up().await;

        // create buffers for the request and response
        let mut rx_buffer = [0; 8192];
        let mut tls_read_buffer = [0; 16640];
        let mut tls_write_buffer = [0; 16640];

        let client_state = TcpClientState::<1, 1024, 1024>::new();
        let tcp_client = TcpClient::new(*stack, &client_state);
        let dns_client = dns::DnsSocket::new(*stack);
        let _tls_config = TlsConfig::new(
            seed,
            &mut tls_read_buffer,
            &mut tls_write_buffer,
            TlsVerify::None,
        );

        '_http_client: {
            // create a new scope to limit the lifetime of the HttpClient and the request
            // scope for request, response, and body. This is to ensure that the request is dropped before the next iteration of the loop.

            let mut http_client = HttpClient::new(&tcp_client, &dns_client);

            let url = time_updater.time_api_url();

            // make the request
            let mut request = match http_client.request(Method::GET, url).await {
                Ok(req) => req,
                Err(e) => {
                    control.leave().await;
                    control.gpio_set(0, false).await; // Turn off the onboard LED
                    warn!(
                        "Failed to make HTTP request, retrying in {:?} seconds: {:?}",
                        time_updater.retry_after_secs,
                        Debug2Format(&e)
                    );
                    Timer::after(Duration::from_secs(time_updater.retry_after_secs)).await;
                    continue '_mainloop;
                }
            };

            let response = match request.send(&mut rx_buffer).await {
                Ok(resp) => resp,
                Err(e) => {
                    control.leave().await;
                    control.gpio_set(0, false).await; // Turn off the onboard LED
                    warn!(
                        "Disconnected from wifi after error. Retrying in {:?} seconds: {:?}",
                        time_updater.retry_after_secs,
                        Debug2Format(&e)
                    );
                    Timer::after(Duration::from_secs(time_updater.retry_after_secs)).await;
                    continue '_mainloop;
                }
            };

            let body = match from_utf8(response.body().read_to_end().await.unwrap()) {
                Ok(b) => b,
                Err(e) => {
                    control.leave().await;
                    control.gpio_set(0, false).await; // Turn off the onboard LED
                    warn!(
                        "Disconnected from wifi after error. Retrying in {:?} seconds: {:?}",
                        time_updater.retry_after_secs,
                        Debug2Format(&e)
                    );
                    Timer::after(Duration::from_secs(time_updater.retry_after_secs)).await;
                    continue '_mainloop;
                }
            };
            info!("Response body: {:?}", &body);

            // parse the response body and update the RTC
            #[derive(Deserialize)]
            struct ApiResponse<'a> {
                datetime: &'a str,
                day_of_week: u8,
            }

            let bytes = body.as_bytes();
            let response: ApiResponse = match serde_json_core::de::from_slice::<ApiResponse>(bytes)
            {
                Ok((output, _used)) => {
                    info!("Datetime: {:?}", output.datetime);
                    info!("Day of week: {:?}", output.day_of_week);
                    output
                }
                Err(e) => {
                    warn!(
                        "Failed to parse response body. Retrying in {:?} seconds: {:?}",
                        time_updater.retry_after_secs,
                        Debug2Format(&e)
                    );
                    Timer::after(Duration::from_secs(time_updater.retry_after_secs)).await;
                    continue '_mainloop;
                }
            };

            // set the RTC
            '_rtc_mutex: {
                let dt: DateTime;
                dt = StringUtils::convert_str_to_datetime(response.datetime, response.day_of_week);

                let mut rtc_guard = RTC_MUTEX.lock().await;
                let rtc = rtc_guard.as_mut().unwrap();

                match rtc.set_datetime(dt) {
                    Ok(_) => {
                        // send an event to the state manager
                        EVENT_CHANNEL.sender().send(Events::RtcUpdated).await;
                    }
                    Err(e) => {
                        warn!(
                            "Failed to set datetime. Retrying in {:?} seconds: {:?}",
                            time_updater.retry_after_secs,
                            Debug2Format(&e)
                        );
                        drop(rtc_guard);
                        Timer::after(Duration::from_secs(time_updater.retry_after_secs)).await;
                        continue '_mainloop;
                    }
                }
            }
        }

        control.leave().await;
        control.gpio_set(0, false).await; // Turn off the onboard LED
        info!("Disconnected from wifi");

        control
            .set_power_management(cyw43::PowerManagementMode::Aggressive)
            .await;

        info!(
            "Waiting for {:?} seconds before reconnecting",
            time_updater.refresh_after_secs
        );

        // wait for the refresh time or the resume signal
        let downtime_timer = Timer::after(Duration::from_secs(time_updater.refresh_after_secs));
        select(downtime_timer, TIME_UPDATER_RESUME_SIGNAL.wait()).await;
    }
}
