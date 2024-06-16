#![allow(async_fn_in_trait)]
//make sure to have a wifi_manager.json file in the classes folder formatted as follows:
//{
//    "ssid": "some_ssid_here",
//    "password": "some_password_here"
//}
//also make sure that build.rs loads the wifi_manager.json file and writes it to wifi_secrets.rs
include!(concat!(env!("OUT_DIR"), "/wifi_secrets.rs"));

use cyw43::ControlError;
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::Stack;
use embassy_rp::gpio::Output;
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_time::{Duration, Timer};
use heapless::String;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

enum WifiState {
    Disconnected,
    Connected,
    Error(ControlError), // Optionally, include an error message
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
        self.ssid = Some(self.convert_str_to_heapless_safe(SSID).unwrap());
        self.password = Some(self.convert_str_to_heapless_safe(PASSWORD).unwrap());
    }

    /// This function converts a &str to a heapless::String<128>. Apparently simple strings are not really woking in embedded systems
    fn convert_str_to_heapless_safe(
        &mut self,
        s: &str,
    ) -> Result<heapless::String<128>, &'static str> {
        let mut heapless_string: heapless::String<128> = heapless::String::new();
        for c in s.chars() {
            if heapless_string.push(c).is_err() {
                return Err("String exceeds capacity");
            }
        }
        Ok(heapless_string)
    }

    /// This function unwraps a heapless::String<128> or returns an empty heapless::String<128> if None.
    fn unwrap_or_default_heapless_string(
        &self,
        s: Option<heapless::String<128>>,
    ) -> heapless::String<128> {
        match s {
            Some(value) => value,            // Directly return the heapless::String<128>
            None => heapless::String::new(), // Return an empty heapless::String if None
        }
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
    // let mut wifi_manager = WifiManager::new(); // Initialize WifiManager
    // wifi_manager.set_credentials(); // Set credentials from wifi_secrets.rs

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
    let (_net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    unwrap!(spawner.spawn(wifi_task(runner)));

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let ssid_str = wifi_manager.unwrap_or_default_heapless_string(wifi_manager.ssid.clone()); // Assuming ssid is Option<heapless::String<128>>
    let password_str =
        wifi_manager.unwrap_or_default_heapless_string(wifi_manager.password.clone()); // Assuming password is Option<heapless::String<128>>

    info!(
        "Joining WPA2 network with SSID: {:?} and password: {:?}",
        ssid_str, password_str
    );

    if let Err(e) = control.join_wpa2(&ssid_str, &password_str).await {
        wifi_manager.set_state(WifiState::Error(e));
    } else {
        wifi_manager.set_state(WifiState::Connected);
    };

    // every 120 seconds, check if we are still connected
    loop {
        Timer::after(Duration::from_secs(120)).await;
        info!("In loop and doing fuck all");
    }
}
