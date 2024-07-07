// we are in an environment with constrained resources, so we do not use the standard library and we define a different entry point.
#![no_std]
#![no_main]

use crate::task::alarm_mgr::AlarmManager;
use crate::task::btn_mgr::{blue_button, green_button, yellow_button};
use crate::task::peripherals::{
    AssignedResources, ButtonResourcesBlue, ButtonResourcesGreen, ButtonResourcesYellow,
    DisplayResources, NeopixelResources, RtcResources, WifiResources,
};
use crate::task::time_updater::connect_and_update_rtc;
use core::cell::RefCell;
 // for WiFi
use defmt::*; // global logger
use embassy_executor::Executor;
use embassy_executor::Spawner;
use embassy_rp::gpio::{self};
use embassy_rp::multicore::{spawn_core1, Stack};
use embassy_rp::rtc::Rtc;
use embassy_rp::{peripherals};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use task::alarm_mgr;
use {defmt_rtt as _, panic_probe as _};

// import the task module (submodule of src)
mod task;

// import the utility module (submodule of src)
mod utility;

// Entry point
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Program start");

    // Initialize the peripherals for the RP2040
    let p = embassy_rp::init(Default::default());
    // and assign the peripherals to the places, where we will use them
    let r = split_resources!(p);

    // Alarm Manager
    let alarm_mgr = AlarmManager::new();

    // Buttons
    spawner.spawn(green_button(spawner, r.btn_green)).unwrap();
    spawner.spawn(blue_button(spawner, r.btn_blue)).unwrap();
    spawner.spawn(yellow_button(spawner, r.btn_yellow)).unwrap();

    // RTC

    // Initialize the RTC in a static cell, we will need it in multiple places
    static RTC: StaticCell<RefCell<Rtc<'static, peripherals::RTC>>> = StaticCell::new();
    let rtc_instance: Rtc<'static, peripherals::RTC> = Rtc::new(r.rtc.rtc_inst);
    let rtc_ref = RTC.init(RefCell::new(rtc_instance));

    // update the RTC
    spawner
        .spawn(connect_and_update_rtc(spawner, r.wifi, rtc_ref))
        .unwrap();

    // Neopixel
    // Note! -> we may need more than one neopixel task eventually, in that case we will need mutexes around the resources
    // i want to keep it simple for now
    // best case will be to have a single task that somehow can handle analog clock, as well as alarm light as well as staying idle
    // maybe we can have a channel to send "a state has changed" signal to the neopixel task and then the task can decide what to do

    // spawn the neopixel tasks, on core1 as opposed to the other tasks
    static mut CORE1_STACK: Stack<4096> = Stack::new();
    static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

    // Channel to send the idle signal to the neopixel tasks
    static ALARM_IDLE_CHANNEL: Channel<CriticalSectionRawMutex, task::alarm_mgr::AlarmState, 1> =
        Channel::new();

    spawn_core1(
        p.CORE1,
        unsafe { &mut *core::ptr::addr_of_mut!(CORE1_STACK) },
        move || {
            let executor1 = EXECUTOR1.init(Executor::new());
            executor1.run(|spawner| {
                spawner
                    .spawn(task::neopixel::analog_clock(
                        spawner,
                        r.neopixel,
                        ALARM_IDLE_CHANNEL.receiver(),
                    ))
                    .unwrap();
            });
        },
    );

    // Display
    // let irqs_display = Irqs;
    // let scl = p.PIN_13;
    // let sda = p.PIN_12;
    // let i2c0 = p.I2C0;
    // initialize_display(&spawner, scl, sda, i2c0, irqs_display).await;

    // static I2C_BUS_CELL: StaticCell<Mutex<NoopRawMutex, I2c<I2C0, Async>>> = StaticCell::new();
    // let scl = p.PIN_13;
    // let sda = p.PIN_12;
    // let mut i2c_config = I2cConfig::default();
    // i2c_config.frequency = 400_000;
    // let i2c_dsp = I2c::new_async(p.I2C0, scl, sda, Irqs, i2c_config);
    // let i2c_dsp_bus: &'static _ = I2C_BUS_CELL.init(Mutex::<NoopRawMutex, _>::new(i2c_dsp));

    // spawner
    //     .spawn(task::display::display(spawner, i2c_dsp_bus))
    //     .unwrap();

    // Main loop, doing very little
    loop {
        if let Ok(dt) = rtc_ref.borrow_mut().now() {
            info!(
                "Main loop: {}-{:02}-{:02} {}:{:02}:{:02}",
                dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second,
            );
        }
        Timer::after(Duration::from_secs(10)).await;

        info!("Sending idle signal to neopixel tasks");
        ALARM_IDLE_CHANNEL
            .sender()
            .send(alarm_mgr::AlarmState::Idle)
            .await;

        Timer::after(Duration::from_secs(10)).await;
    }
}
