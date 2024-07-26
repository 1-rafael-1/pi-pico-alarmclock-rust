//! # Main
//! This is the main entry point of the program.
//! we are in an environment with constrained resources, so we do not use the standard library and we define a different entry point.
#![no_std]
#![no_main]

use crate::task::buttons::{blue_button, green_button, yellow_button};
use crate::task::dfplayer::sound;
use crate::task::display::display;
use crate::task::persisted_alarm_time::read_persisted_alarm_time;
use crate::task::power::{usb_power, vsys_voltage};
use crate::task::resources::*;
use crate::task::state::*;
use crate::task::time_updater::time_updater;
use core::cell::RefCell;
use defmt::*;
use embassy_executor::{Executor, Spawner};
use embassy_rp::multicore::{spawn_core1, Stack};
use embassy_rp::peripherals;
use embassy_rp::rtc::Rtc;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// import the task module (submodule of src)
mod task;

// import the utility module (submodule of src)
mod utility;

/// Entry point
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Program start");

    // Initialize the peripherals for the RP2040
    let p = embassy_rp::init(Default::default());
    // and assign the peripherals to the places, where we will use them
    let r = split_resources!(p);

    // configure, which tasks to spawn. For a production build we need all tasks, for troubleshooting we can disable some
    // the tasks are all spawned in main.rs, so we can disable them here
    // clutter in the output aside, the binary size is conveniently reduced by disabling tasks
    let mut task_config = TaskConfig::new();
    task_config.time_updater = true;
    task_config.btn_green = true;
    task_config.btn_blue = true;
    task_config.btn_yellow = true;
    task_config.neopixel = false;
    task_config.display = false;
    task_config.dfplayer = false;
    task_config.usb_power = true;
    task_config.vsys_voltage = true;
    task_config.persisted_alarm_time = true;

    // RTC
    // Initialize the RTC in a static cell, we will need it in multiple places
    static RTC: StaticCell<RefCell<Rtc<'static, peripherals::RTC>>> = StaticCell::new();
    let rtc_instance: Rtc<'static, peripherals::RTC> = Rtc::new(r.rtc.rtc_inst);
    let rtc_ref = RTC.init(RefCell::new(rtc_instance));

    // Orchestrate
    // there is no main loop, the tasks are spawned and run in parallel
    // orchestrating the tasks is done here:
    spawner.spawn(orchestrate(spawner, rtc_ref)).unwrap();

    // PersistedAlarmTime
    if task_config.persisted_alarm_time {
        spawner
            .spawn(read_persisted_alarm_time(spawner, r.flash))
            .unwrap();
    }

    // Power
    if task_config.usb_power {
        spawner.spawn(usb_power(spawner, r.vbus_power)).unwrap();
    };

    if task_config.vsys_voltage {
        spawner
            .spawn(vsys_voltage(spawner, r.vsys_resources))
            .unwrap();
    }

    // Buttons
    if task_config.btn_green {
        spawner.spawn(green_button(spawner, r.btn_green)).unwrap();
    };
    if task_config.btn_blue {
        spawner.spawn(blue_button(spawner, r.btn_blue)).unwrap();
    };
    if task_config.btn_yellow {
        spawner.spawn(yellow_button(spawner, r.btn_yellow)).unwrap();
    };

    // update the RTC
    if task_config.time_updater {
        spawner
            .spawn(time_updater(spawner, r.wifi, rtc_ref))
            .unwrap();
    }

    // Neopixel
    // Note! -> we may need more than one neopixel task eventually, in that case we will need mutexes around the resources
    // i want to keep it simple for now

    // the neopixel task will be spawned on core1, because it will run in parallel to the other tasks and it may block
    // spawn the neopixel tasks, on core1 as opposed to the other tasks
    static mut CORE1_STACK: Stack<4096> = Stack::new();
    static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

    spawn_core1(
        p.CORE1,
        unsafe { &mut *core::ptr::addr_of_mut!(CORE1_STACK) },
        move || {
            let executor1 = EXECUTOR1.init(Executor::new());
            executor1.run(|spawner| {
                if task_config.neopixel {
                    spawner
                        .spawn(task::neopixel::analog_clock(spawner, r.neopixel))
                        .unwrap();
                }
            });
        },
    );

    // Display
    if task_config.display {
        spawner.spawn(display(spawner, r.display)).unwrap();
    }

    // DFPlayer
    if task_config.dfplayer {
        spawner.spawn(sound(spawner, r.dfplayer)).unwrap();
    }
}
