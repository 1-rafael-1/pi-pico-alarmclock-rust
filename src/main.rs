// we are in an environment with constrained resources, so we do not use the standard library and we define a different entry point.
#![no_std]
#![no_main]

use crate::classes::wifi_mgr::WifiManager; // WifiManager
use cyw43_pio::PioSpi; // for WiFi
use defmt::*; // global logger
use embassy_executor::Spawner; // executor
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{self, Input}; // gpio
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_time::{Duration, Timer}; // time
use gpio::{Level, Output}; // gpio output
use {defmt_rtt as _, panic_probe as _}; // panic handler

// import the classes module (submodule of src)
mod classes;

// Entry point
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Program start");

    // Initialize the peripherals for the RP2040
    let peripherals = embassy_rp::init(Default::default());

    // Initialize one LED on pin 25
    // let mut led = Output::new(peripherals.PIN_25, Level::Low);

    // buttons
    // green_button
    info!("init green button");
    let green_button_input = Input::new(peripherals.PIN_20, gpio::Pull::Up);
    spawner
        .spawn(classes::btn_mgr::green_button(spawner, green_button_input))
        .unwrap();

    //blue_button
    info!("init blue button");
    let blue_button_input = Input::new(peripherals.PIN_21, gpio::Pull::Up);
    spawner
        .spawn(classes::btn_mgr::blue_button(spawner, blue_button_input))
        .unwrap();

    //yellow_button
    info!("init yellow button");
    let yellow_button_input = Input::new(peripherals.PIN_22, gpio::Pull::Up);
    spawner
        .spawn(classes::btn_mgr::yellow_button(
            spawner,
            yellow_button_input,
        ))
        .unwrap();

    //wifi
    // Setup for WiFi connection
    info!("init wifi");
    let pwr = Output::new(peripherals.PIN_23, Level::Low);
    let cs = Output::new(peripherals.PIN_25, Level::High);
    let mut pio = Pio::new(peripherals.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        peripherals.PIN_24,
        peripherals.PIN_29,
        peripherals.DMA_CH0,
    );
    bind_interrupts!(struct Irqs {
        PIO0_IRQ_0 => InterruptHandler<PIO0>;
    });

    // Initialize WifiManager
    let mut wifi_manager = WifiManager::new();

    // Call connect_wifi with the necessary parameters
    spawner
        .spawn(classes::wifi_mgr::connect_wifi(
            spawner,
            wifi_manager,
            pwr,
            spi,
        ))
        .unwrap();

    loop {
        info!("main loop");
        Timer::after(Duration::from_secs(10)).await;
        // info!("led on!");
        // led.set_high();
        // Timer::after(Duration::from_secs(20)).await;

        // info!("led off!");
        // led.set_low();
        // Timer::after(Duration::from_secs(20)).await;
    }
}
