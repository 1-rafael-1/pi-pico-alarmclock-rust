#![no_std]
#![no_main]
#![allow(async_fn_in_trait)]

//make sure to have a wifi_manager.json file in the classes folder formatted as follows:
//{
//    "ssid": "some_ssid_here",
//    "password": "some_password_here"
//}
include!("wifi_secrets.rs");

use core::str;

use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::Stack;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use heapless::String;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

enum WifiState {
    Disconnected,
    Connected,
    Error(String), // Optionally, include an error message
}

struct WifiManager {
    state: WifiState,
    ssid: Option<String>,
    password: Option<String>,
}

impl WifiManager {
    fn new() -> Self {
        WifiManager {
            state: WifiState::Disconnected,
            ssid: Some(SSID.to_string()),
            password: Some(PASSWORD.to_string()),
        }
    }

    fn set_state(&mut self, new_state: WifiState) {
        self.state = new_state;
    }

    fn set_credentials(&mut self, ssid: String, password: String) {
        self.ssid = Some(ssid);
        self.password = Some(password);
    }

    bind_interrupts!(struct Irqs {
        PIO0_IRQ_0 => InterruptHandler<PIO0>;
    });

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
    async fn connect(spawner: Spawner) {
        info!("Initializing WifiManager...");
        let mut wifi_manager = WifiManager::new(); // Initialize WifiManager

        let p = embassy_rp::init(Default::default());

        //let fw = include_bytes!("../../../../cyw43-firmware/43439A0.bin");
        //let clm = include_bytes!("../../../../cyw43-firmware/43439A0_clm.bin");
        // To make flashing faster for development, you may want to flash the firmwares independently
        // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
        //     probe-rs download 43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
        //     probe-rs download 43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
        let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
        let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

        let pwr = Output::new(p.PIN_23, Level::Low);
        let cs = Output::new(p.PIN_25, Level::High);
        let mut pio = Pio::new(p.PIO0, Irqs);
        let spi = PioSpi::new(
            &mut pio.common,
            pio.sm0,
            pio.irq0,
            cs,
            p.PIN_24,
            p.PIN_29,
            p.DMA_CH0,
        );

        static STATE: StaticCell<cyw43::State> = StaticCell::new();
        let state = STATE.init(cyw43::State::new());
        let (_net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
        unwrap!(spawner.spawn(wifi_task(runner)));

        control.init(clm).await;
        control
            .set_power_management(cyw43::PowerManagementMode::PowerSave)
            .await;
    }
}
