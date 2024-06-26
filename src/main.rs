// we are in an environment with constrained resources, so we do not use the standard library and we define a different entry point.
#![no_std]
#![no_main]

use crate::classes::time_updater::TimeUpdater;
use core::cell::RefCell;
use cyw43_pio::PioSpi; // for WiFi
use defmt::*; // global logger
use embassy_executor::Spawner; // executor
use embassy_rp::gpio::{self, Input};
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::rtc::Rtc;
use embassy_rp::{bind_interrupts, peripherals};
use embassy_time::{Duration, Timer}; // time
use gpio::{Level, Output};
use static_cell::StaticCell; // gpio output
use {defmt_rtt as _, panic_probe as _}; // panic handler

// import the classes module (submodule of src)
mod classes;

// import the utility module (submodule of src)
mod utility;

// Entry point
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Program start");

    // Initialize the peripherals for the RP2040
    let p = embassy_rp::init(Default::default());
    // bind the interrupts
    bind_interrupts!(struct Irqs {
        PIO0_IRQ_0 => InterruptHandler<PIO0>;
    });

    // buttons
    // green_button
    info!("init green button");
    let green_button_input = Input::new(p.PIN_20, gpio::Pull::Up);
    spawner
        .spawn(classes::btn_mgr::green_button(spawner, green_button_input))
        .unwrap();

    //blue_button
    info!("init blue button");
    let blue_button_input = Input::new(p.PIN_21, gpio::Pull::Up);
    spawner
        .spawn(classes::btn_mgr::blue_button(spawner, blue_button_input))
        .unwrap();

    //yellow_button
    info!("init yellow button");
    let yellow_button_input = Input::new(p.PIN_22, gpio::Pull::Up);
    spawner
        .spawn(classes::btn_mgr::yellow_button(
            spawner,
            yellow_button_input,
        ))
        .unwrap();

    // Real Time Clock
    // Setup for WiFi connection and RTC update
    info!("init wifi");
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

    // Initialize the RTC in a static cell to be used in the time_updater module
    static RTC: StaticCell<RefCell<Rtc<'static, peripherals::RTC>>> = StaticCell::new();
    let rtc_instance: Rtc<'static, peripherals::RTC> = Rtc::new(p.RTC);
    let rtc_ref = RTC.init(RefCell::new(rtc_instance));

    // Initialize TimeUpdater
    let time_updater = TimeUpdater::new();

    // Call connect_wifi with the necessary parameters
    spawner
        .spawn(classes::time_updater::connect_and_update_rtc(
            spawner,
            time_updater,
            pwr,
            spi,
            rtc_ref,
        ))
        .unwrap();

    loop {
        info!("main loop");
        if let Ok(dt) = rtc_ref.borrow_mut().now() {
            info!(
                "Task A: Now: {}-{:02}-{:02} {}:{:02}:{:02}",
                dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second,
            );
        }
        Timer::after(Duration::from_secs(10)).await;
    }
}
