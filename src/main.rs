//! # Main
//! This is the main entry point of the program.
//! we are in an environment with constrained resources, so we do not use the standard library and we define a different entry point.
#![no_std]
#![no_main]

use crate::task::alarm_settings::alarm_settings_handler;
use crate::task::buttons::{blue_button_handler, green_button_handler, yellow_button_handler};
use crate::task::display::display_handler;
use crate::task::orchestrate::{orchestrator, scheduler};
use crate::task::power::{usb_power_detector, vsys_voltage_reader};
use crate::task::resources::*;
use crate::task::sound::sound_handler;
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
    let task_config = TaskConfig::new();

    // RTC
    // Initialize the RTC in a static cell, we will need it in multiple places
    static RTC: StaticCell<RefCell<Rtc<'static, peripherals::RTC>>> = StaticCell::new();
    let rtc_instance: Rtc<'static, peripherals::RTC> = Rtc::new(r.real_time_clock.rtc);
    let rtc_ref = RTC.init(RefCell::new(rtc_instance));

    // Orchestrate
    // there is no main loop, the tasks are spawned and run in parallel
    // orchestrating the tasks is done here:
    if task_config.orchestrator {
        spawner.spawn(orchestrator(spawner)).unwrap();
        spawner.spawn(scheduler(spawner, rtc_ref)).unwrap();
    }

    // Alarm settings
    if task_config.alarm_settings_handler {
        spawner
            .spawn(alarm_settings_handler(spawner, r.flash))
            .unwrap();
    };

    // Power
    if task_config.usb_power {
        spawner
            .spawn(usb_power_detector(spawner, r.vbus_power))
            .unwrap();
    };

    if task_config.vsys_voltage_reader {
        spawner
            .spawn(vsys_voltage_reader(spawner, r.vsys_resources))
            .unwrap();
    };

    // Buttons
    if task_config.btn_green_handler {
        spawner
            .spawn(green_button_handler(spawner, r.btn_green))
            .unwrap();
    };
    if task_config.btn_blue_handler {
        spawner
            .spawn(blue_button_handler(spawner, r.btn_blue))
            .unwrap();
    };
    if task_config.btn_yellow_handler {
        spawner
            .spawn(yellow_button_handler(spawner, r.btn_yellow))
            .unwrap();
    };

    // update the RTC
    if task_config.time_updater {
        spawner
            .spawn(time_updater(spawner, r.wifi, rtc_ref))
            .unwrap();
    }

    // Neopixel
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
                if task_config.light_effects_handler {
                    spawner
                        .spawn(task::light_effects::light_effects_handler(
                            spawner, r.neopixel,
                        ))
                        .unwrap();
                }
            });
        },
    );

    // Display
    if task_config.display_handler {
        spawner
            .spawn(display_handler(spawner, r.display, rtc_ref))
            .unwrap();
    }

    // DFPlayer
    if task_config.sound_handler {
        spawner.spawn(sound_handler(spawner, r.dfplayer)).unwrap();
    }
}

/// This struct is used to configure which tasks are enabled
/// This is useful for troubleshooting, as we can disable tasks to reduce the binary size
/// and clutter in the output.
/// Also, we can disable tasks that are not needed for the current development stage and also test tasks in isolation.
/// For a production build we will need all tasks enabled
pub struct TaskConfig {
    pub btn_green_handler: bool,
    pub btn_blue_handler: bool,
    pub btn_yellow_handler: bool,
    pub time_updater: bool,
    pub light_effects_handler: bool,
    pub display_handler: bool,
    pub sound_handler: bool,
    pub usb_power: bool,
    pub vsys_voltage_reader: bool,
    pub alarm_settings_handler: bool,
    pub orchestrator: bool,
}

impl Default for TaskConfig {
    fn default() -> Self {
        TaskConfig {
            btn_green_handler: true,
            btn_blue_handler: true,
            btn_yellow_handler: true,
            time_updater: true,
            light_effects_handler: true,
            display_handler: true,
            sound_handler: true,
            usb_power: true,
            vsys_voltage_reader: true,
            alarm_settings_handler: true,
            orchestrator: true,
        }
    }
}

impl TaskConfig {
    pub fn new() -> Self {
        TaskConfig::default()
    }
}
