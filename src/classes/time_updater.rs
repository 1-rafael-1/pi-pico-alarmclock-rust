#![allow(async_fn_in_trait)]
include!(concat!(env!("OUT_DIR"), "/wifi_secrets.rs"));
// populate constants SSID and PASSWORD
// make sure to have a wifi_manager.json file in the config folder formatted as follows:
// {
//     "ssid": "some_ssid_here",
//     "password": "some_password_here"
// }
// also make sure that build.rs loads the wifi_manager.json file and writes it to wifi_secrets.rs

include!(concat!(env!("OUT_DIR"), "/time_api_config.rs"));
// populate constant TIME_SERVER_URL
// make sure to have a time_api_config.json file in the config folder formatted as follows:

use core::str::from_utf8;
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::{
    dns,
    tcp::client::{TcpClient, TcpClientState},
    Config, Stack, StackResources,
};
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::Output;
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::rtc::Rtc;
use embassy_time::{Duration, Instant, Timer};
use rand::RngCore;
use reqwless::client::HttpClient;
use reqwless::client::TlsConfig;
use reqwless::client::TlsVerify;
use reqwless::request::Method;
use serde::Deserialize;
use serde_json_core;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

pub struct TimeUpdater {
    ssid: &'static str,
    password: &'static str,
    time_api_url: &'static str,
    rtc_is_set: bool,
    rtc_last_update: Instant,
}

impl TimeUpdater {
    pub fn new() -> Self {
        let mut manager = TimeUpdater {
            ssid: "",
            password: "",
            time_api_url: "",
            rtc_is_set: false,
            rtc_last_update: Instant::now(),
        };
        manager.set_credentials();
        manager.set_time_api_url();
        manager
    }

    fn set_credentials(&mut self) {
        self.ssid = SSID;
        self.password = PASSWORD;
    }

    fn get_credentials(&self) -> (&str, &str) {
        (self.ssid, self.password)
    }

    fn set_time_api_url(&mut self) {
        self.time_api_url = TIME_SERVER_URL;
    }

    fn get_time_api_url(&self) -> &str {
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
pub async fn connect_and_update_rtc(
    spawner: Spawner,
    mut time_updater: TimeUpdater,
    pwr: Output<'static>,
    spi: PioSpi<'static, PIO0, 0, DMA_CH0>,
    rtc: Rtc,
) {
    let secs_to_wait = 21600; // 6 hours
    let secs_to_wait_retry = 30;

    let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    unwrap!(spawner.spawn(wifi_task(runner)));

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let config = Config::dhcpv4(Default::default());
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

    loop {
        let (ssid, password) = time_updater.get_credentials();
        info!(
            "Joining WPA2 network with SSID: {:?} and password: {:?}",
            &ssid, &password
        );
        match control.join_wpa2(&ssid, &password).await {
            Ok(_) => {
                control.gpio_set(0, true).await; // Turn on the onboard LED
                info!("Connected to wifi");
            }
            Err(e) => {
                info!("Error connecting to wifi: {}", e.status);
                control.leave().await;
                control.gpio_set(0, false).await; // Turn off the onboard LED
                info!(
                    "Disconnected from wifi after error. Retrying in {:?} seconds",
                    secs_to_wait_retry
                );
                Timer::after(Duration::from_secs(secs_to_wait_retry)).await;
                continue;
            }
        }

        info!("waiting for DHCP...");
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
            info!(
                "Disconnected from wifi after error. Retrying in {:?} seconds",
                secs_to_wait_retry
            );
            Timer::after(Duration::from_secs(secs_to_wait_retry)).await;
            continue;
        }
        info!("DHCP is now up!");

        info!("waiting for link up...");
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
            info!(
                "Disconnected from wifi after error. Retrying in {:?} seconds",
                secs_to_wait_retry
            );
            Timer::after(Duration::from_secs(secs_to_wait_retry)).await;
            continue;
        }
        info!("Link is up!");

        info!("waiting for stack to be up...");
        stack.wait_config_up().await;
        info!("Stack is up!");

        info!("Preparing request to timeapi.io");
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

        {
            // create a new scope to limit the lifetime of the HttpClient and the request
            // scope for request, response, and body. This is to ensure that the request is dropped before the next iteration of the loop.

            let mut http_client = HttpClient::new_with_tls(&tcp_client, &dns_client, tls_config);
            info!("HttpClient created");

            let url = time_updater.get_time_api_url();

            info!("Making request");
            let mut request = match http_client.request(Method::GET, url).await {
                Ok(req) => req,
                Err(e) => {
                    error!("Failed to make HTTP request: {:?}", e);
                    control.leave().await;
                    control.gpio_set(0, false).await; // Turn off the onboard LED
                    info!(
                        "Disconnected from wifi after error. Retrying in {:?} seconds",
                        secs_to_wait_retry
                    );
                    Timer::after(Duration::from_secs(secs_to_wait_retry)).await;
                    continue;
                }
            };
            let response = match request.send(&mut rx_buffer).await {
                Ok(resp) => resp,
                Err(e) => {
                    error!("Failed to send HTTP request");
                    control.leave().await;
                    control.gpio_set(0, false).await; // Turn off the onboard LED
                    info!(
                        "Disconnected from wifi after error. Retrying in {:?} seconds",
                        secs_to_wait_retry
                    );
                    Timer::after(Duration::from_secs(secs_to_wait_retry)).await;
                    continue;
                }
            };
            let body = match from_utf8(response.body().read_to_end().await.unwrap()) {
                Ok(b) => b,
                Err(e) => {
                    error!("Failed to read response body");
                    control.leave().await;
                    control.gpio_set(0, false).await; // Turn off the onboard LED
                    info!(
                        "Disconnected from wifi after error. Retrying in {:?} seconds",
                        secs_to_wait_retry
                    );
                    Timer::after(Duration::from_secs(secs_to_wait_retry)).await;
                    continue;
                }
            };
            info!("Response body: {:?}", &body);

            // parse the response body and update the RTC
            #[derive(Deserialize)]
            struct ApiResponse<'a> {
                datetime: &'a str,
            }

            let bytes = body.as_bytes();
            match serde_json_core::de::from_slice::<ApiResponse>(bytes) {
                Ok((output, _used)) => {
                    info!("Datetime: {:?}", output.datetime);
                    // set the RTC
                    rtc.set_datetime(output.datetime).unwrap();
                }
                Err(e) => {
                    error!("Failed to parse response body");
                    return; // ToDo
                }
            }
        } // end of scope

        control.leave().await;
        control.gpio_set(0, false).await; // Turn off the onboard LED
        info!("Disconnected from wifi");

        info!("Waiting for {:?} seconds before reconnecting", secs_to_wait);
        Timer::after(Duration::from_secs(secs_to_wait)).await;
    }
}
