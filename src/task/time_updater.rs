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

use crate::task::resources::{Irqs, WifiResources};
use crate::task::task_messages::{Events, EVENT_CHANNEL};
use crate::utility::string_utils::StringUtils;
use crate::RtcResources;
use core::borrow::BorrowMut;
use core::cell::RefCell;
use core::str::from_utf8;
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::DhcpConfig;
use embassy_net::{
    dns,
    tcp::client::{TcpClient, TcpClientState},
    Config, Stack, StackResources,
};
use embassy_rp::gpio::Level;
use embassy_rp::gpio::Output;
use embassy_rp::peripherals;
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::Pio;
use embassy_rp::rtc::Rtc;
use embassy_rp::{clocks::RoscRng, rtc::DateTime};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{with_timeout, Duration, Timer};
use rand::{Error, RngCore};
use reqwless::client::HttpClient;
use reqwless::client::TlsConfig;
use reqwless::client::TlsVerify;
use reqwless::request::Method;
use serde::Deserialize;
use serde_json_core;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

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
async fn net_task(stack: &'static Stack<cyw43::NetDriver<'static>>) -> ! {
    stack.run().await
}
#[embassy_executor::task]
pub async fn time_updater(spawner: Spawner, r: WifiResources, t: RtcResources) {
    info!("time updater task started");

    info!("init rtc");
    // initialize the rtc and put it into a mutex
    {
        *(RTC_MUTEX.lock().await) = Some(Rtc::new(t.rtc));
    }

    info!("init wifi");
    let pwr = Output::new(r.pwr_pin, Level::Low);
    let cs = Output::new(r.cs_pin, Level::High);
    let mut pio = Pio::new(r.pio_sm, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        r.dio_pin,
        r.clk_pin,
        r.dma_ch,
    );

    let time_updater = TimeUpdater::new();

    let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());

    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;

    unwrap!(spawner.spawn(wifi_task(runner)));

    info!("init control");
    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let mut default_config: DhcpConfig = Default::default();
    default_config.hostname = Some("alarmclck".try_into().unwrap());
    let config = Config::dhcpv4(default_config);

    // random seed
    let mut rng = RoscRng;
    let seed = rng.next_u64();

    // Initialize the network stack
    static STACK: StaticCell<Stack<cyw43::NetDriver<'static>>> = StaticCell::new();
    static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
    let stack = &*STACK.init(Stack::new(
        net_device,
        config,
        RESOURCES.init(StackResources::<5>::new()),
        seed,
    ));

    unwrap!(spawner.spawn(net_task(stack)));

    info!("starting loop");
    '_mainloop: loop {
        // get the wifi credentials
        let (ssid, password) = time_updater.credentials();
        info!(
            "Joining WPA2 network with SSID: {:?} and password: {:?}",
            &ssid, &password
        );

        // Join the network
        let join_result = with_timeout(
            time_updater.timeout_duration,
            control.join_wpa2(&ssid, &password),
        )
        .await;
        match join_result {
            Ok(Ok(_)) => {
                control.gpio_set(0, true).await; // Turn on the onboard LED
                info!("Connected to wifi");
            }
            Ok(Err(e)) => {
                error!("Error connecting to wifi: {}", Debug2Format(&e));
                control.leave().await;
                control.gpio_set(0, false).await; // Turn off the onboard LED
                Timer::after(Duration::from_secs(time_updater.retry_after_secs)).await;
                continue;
            }
            Err(_) => {
                error!("Timeout while trying to connect to wifi");
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
            error!(
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
            error!(
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
        let tcp_client = TcpClient::new(stack, &client_state);
        let dns_client = dns::DnsSocket::new(stack);
        let tls_config = TlsConfig::new(
            seed,
            &mut tls_read_buffer,
            &mut tls_write_buffer,
            TlsVerify::None,
        );

        '_http_client: {
            // create a new scope to limit the lifetime of the HttpClient and the request
            // scope for request, response, and body. This is to ensure that the request is dropped before the next iteration of the loop.

            let mut http_client = HttpClient::new_with_tls(&tcp_client, &dns_client, tls_config);

            let url = time_updater.time_api_url();

            // make the request
            let mut request = match http_client.request(Method::GET, url).await {
                Ok(req) => req,
                Err(e) => {
                    control.leave().await;
                    control.gpio_set(0, false).await; // Turn off the onboard LED
                    error!(
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
                    error!(
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
                    error!(
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
                    error!(
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
                let mut rtc = rtc_guard.as_mut().unwrap();

                match rtc.set_datetime(dt) {
                    Ok(_) => {
                        // send an event to the state manager
                        EVENT_CHANNEL.sender().send(Events::RtcUpdated).await;
                    }
                    Err(e) => {
                        error!(
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

        info!(
            "Waiting for {:?} seconds before reconnecting",
            time_updater.refresh_after_secs
        );
        Timer::after(Duration::from_secs(time_updater.refresh_after_secs)).await;
    }
}
