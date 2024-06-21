#![allow(async_fn_in_trait)]
// make sure to have a wifi_manager.json file in the config folder formatted as follows:
// {
//     "ssid": "some_ssid_here",
//     "password": "some_password_here"
// }
// also make sure that build.rs loads the wifi_manager.json file and writes it to wifi_secrets.rs
include!(concat!(env!("OUT_DIR"), "/wifi_secrets.rs"));

use crate::utility::string_utils::StringUtils;
use core::str::from_utf8;
//use cyw43::{ControlError, State};
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
use heapless::String;
use rand::RngCore;
use reqwless::client::HttpClient;
use reqwless::client::TlsConfig;
use reqwless::client::TlsVerify;
use reqwless::request::Method;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

//use reqwless::client::TlsConfig;

enum WifiState {
    Disconnected,
    Connected,
    Error,
}

pub struct WifiManager {
    state: WifiState,
    ssid: Option<String<128>>,
    password: Option<String<128>>,
}

impl WifiManager {
    pub fn new() -> Self {
        let mut manager = WifiManager {
            state: WifiState::Disconnected,
            ssid: None,
            password: None,
        };
        manager.set_credentials();
        manager
    }

    fn set_state(&mut self, new_state: WifiState) {
        self.state = new_state;
    }

    pub fn get_state(&self) -> &WifiState {
        &self.state
    }

    fn set_credentials(&mut self) {
        self.ssid = Some(StringUtils::convert_str_to_heapless_safe(SSID).unwrap());
        self.password = Some(StringUtils::convert_str_to_heapless_safe(PASSWORD).unwrap());
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
pub async fn connect_wifi(
    spawner: Spawner,
    mut wifi_manager: WifiManager,
    pwr: Output<'static>,
    spi: PioSpi<'static, PIO0, 0, DMA_CH0>,
) {
    // let fw = include_bytes!("../../../../cyw43-firmware/43439A0.bin");
    // let clm = include_bytes!("../../../../cyw43-firmware/43439A0_clm.bin");
    // To make flashing faster for development, you may want to flash the firmwares independently
    // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    //     probe-rs download 43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
    //     probe-rs download 43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
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

    let ssid_str = StringUtils::unwrap_or_default_heapless_string(wifi_manager.ssid.clone()); // Assuming ssid is Option<heapless::String<128>>
    let password_str =
        StringUtils::unwrap_or_default_heapless_string(wifi_manager.password.clone()); // Assuming password is Option<heapless::String<128>>

    info!(
        "Joining WPA2 network with SSID: {:?} and password: {:?}",
        ssid_str, password_str
    );
    match control.join_wpa2(&ssid_str, &password_str).await {
        Ok(_) => {
            wifi_manager.set_state(WifiState::Connected);
            info!("Connected to wifi");
        }
        Err(e) => {
            wifi_manager.set_state(WifiState::Error);
            info!("Error connecting to wifi: {}", e.status);
        }
    }

    // stuff for the http request
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

    // Wait for DHCP, not necessary when using static IP
    info!("waiting for DHCP...");
    while !stack.is_config_up() {
        Timer::after_millis(100).await;
    }
    info!("DHCP is now up!");

    // see if we are connected to the network, if not, wait until we are
    info!("waiting for link up...");
    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
    info!("Link is up!");

    // wait for the stack to be up
    info!("waiting for stack to be up...");
    stack.wait_config_up().await;
    info!("Stack is up!");

    // make the web request
    let mut rx_buffer = [0; 8192];
    let mut tls_read_buffer = [0; 16640];
    let mut tls_write_buffer = [0; 16640];

    info!("Making request to timeapi.io");
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
    let url = "https://timeapi.io/api/Time/current/zone?timeZone=Europe/Berlin";
    info!("URL: {:?}", url);
    info!("Making request");
    // Replace .unwrap() when making the request
    // let mut request = http_client.request(Method::GET, &url).await.unwrap();
    let mut request = match http_client.request(Method::GET, url).await {
        Ok(req) => req,
        Err(e) => {
            error!("Failed to make HTTP request: {:?}", e);
            return; // Exit the function or handle the error appropriately
        }
    };
    info!("Reading response");
    let response = request.send(&mut rx_buffer).await.unwrap();
    let body = from_utf8(response.body().read_to_end().await.unwrap()).unwrap();
    info!("Response body: {:?}", &body);

    // we can end this task here, as we are not doing anything else
}

// #[embassy_executor::task]
// pub async fn get_time_from_service(stack: &'static Stack<cyw43::NetDriver<'static>>) {
//     let mut rx_buffer = [0; 8192];
//     let mut tls_read_buffer = [0; 8192];
//     let mut tls_write_buffer = [0; 8192];

//     // see if we are connected to the network, if not, wait until we are
//     loop {
//         if stack.is_link_up() {
//             break;
//         }
//         Timer::after(Duration::from_millis(500)).await;
//     }

//     stack.wait_config_up().await;

//     let client_state = TcpClientState::<1, 1024, 1024>::new();
//     let tcp_client = TcpClient::new(&stack, &client_state);
//     let dns_client = dns::DnsSocket::new(&stack);
//     let mut http_client = HttpClient::new(&tcp_client, &dns_client);
//     let url = "https://timeapi.io/api/Time/current/zone?timeZone=Europe/Berlin";
//     // Before the fix:
//     // let mut request = http_client
//     //     .request(Method::GET, url)
//     //     .await
//     //     .unwrap()
//     //     .send(&mut rx_buffer)
//     //     .await
//     //     .unwrap();

//     // After the fix:
//     // First, separate the creation of the request from sending it.
//     let mut request_builder = http_client.request(Method::GET, url).await.unwrap();

//     // Then, send the request and await the response.
//     let mut request = request_builder.send(&mut rx_buffer).await.unwrap();

//     // Now you can safely use `request`.
//     let response = request.body().read_to_end().await.unwrap();
//     info!("Response: {:?}", response);
// }
