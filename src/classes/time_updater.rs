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

use core::{str::from_utf8, time};
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
use embassy_time::{Duration, Timer};
use rand::RngCore;
use reqwless::client::HttpClient;
use reqwless::client::TlsConfig;
use reqwless::client::TlsVerify;
use reqwless::request::Method;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

enum WifiState {
    Disconnected,
    Connected,
    Error,
}

pub struct TimeUpdater {
    state: WifiState,
    ssid: &'static str,
    password: &'static str,
    time_api_url: &'static str,
}

impl TimeUpdater {
    pub fn new() -> Self {
        let mut manager = TimeUpdater {
            state: WifiState::Disconnected,
            ssid: "",
            password: "",
            time_api_url: "",
        };
        manager.set_credentials();
        manager.set_time_api_url();
        manager
    }

    fn set_state(&mut self, new_state: WifiState) {
        self.state = new_state;
    }

    fn get_state(&self) -> &WifiState {
        &self.state
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
    mut wifi_manager: TimeUpdater,
    pwr: Output<'static>,
    spi: PioSpi<'static, PIO0, 0, DMA_CH0>,
) {
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

    let (ssid, password) = wifi_manager.get_credentials();
    info!(
        "Joining WPA2 network with SSID: {:?} and password: {:?}",
        &ssid, &password
    );
    match control.join_wpa2(&ssid, &password).await {
        Ok(_) => {
            wifi_manager.set_state(WifiState::Connected);
            info!("Connected to wifi");
        }
        Err(e) => {
            wifi_manager.set_state(WifiState::Error);
            info!("Error connecting to wifi: {}", e.status);
        }
    }

    let config = Config::dhcpv4(Default::default());
    let mut rng = RoscRng;
    let seed = rng.next_u64();
    // Initialize the stack
    static STACK: StaticCell<Stack<cyw43::NetDriver<'static>>> = StaticCell::new();
    static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
    let stack = &*STACK.init(Stack::new(
        net_device,
        config,
        RESOURCES.init(StackResources::<5>::new()),
        seed,
    ));

    unwrap!(spawner.spawn(net_task(stack)));

    info!("waiting for DHCP...");
    while !stack.is_config_up() {
        Timer::after_millis(100).await;
    }
    info!("DHCP is now up!");

    info!("waiting for link up...");
    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
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

    //let mut http_client = HttpClient::new(&tcp_client, &dns_client);
    let mut http_client = HttpClient::new_with_tls(&tcp_client, &dns_client, tls_config);

    info!("HttpClient created");
    let url = wifi_manager.get_time_api_url();

    info!("Making request");
    let mut request = match http_client.request(Method::GET, url).await {
        Ok(req) => req,
        Err(e) => {
            error!("Failed to make HTTP request: {:?}", e);
            return; // ToDo
        }
    };
    let response = request.send(&mut rx_buffer).await.unwrap();
    let body = from_utf8(response.body().read_to_end().await.unwrap()).unwrap();
    info!("Response body: {:?}", &body);
}
