// we are in an environment with constrained resources, so we do not use the standard library and we define a different entry point.
#![no_std]
#![no_main]

use defmt::*; // global logger
use embassy_executor::Spawner; // executor
use embassy_rp::gpio::{self, Input}; // gpio
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
    let mut led = Output::new(peripherals.PIN_25, Level::Low);

    // buttons
    // green_button
    let green_button_input = Input::new(peripherals.PIN_20, gpio::Pull::Up); // initialize the green button on pin 20
    spawner
        .spawn(classes::btn_mgr::green_button(spawner, green_button_input))
        .unwrap(); // spawn the green_button task

    loop {
        info!("led on!");
        led.set_high();
        Timer::after(Duration::from_secs(20)).await;

        info!("led off!");
        led.set_low();
        Timer::after(Duration::from_secs(20)).await;
    }
}
