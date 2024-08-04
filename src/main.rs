//! # Main
//! This is the main entry point of the program.
//! we are in an environment with constrained resources, so we do not use the standard library and we define a different entry point.
#![no_std]
#![no_main]

use crate::task::alarm_settings::manage_alarm_settings;
use crate::task::buttons::{blue_button, green_button, yellow_button};
use crate::task::dfplayer::sound;
use crate::task::display::display;
use crate::task::orchestrate::{minute_timer, orchestrate};
use crate::task::power::{usb_power, vsys_voltage};
use crate::task::resources::*;
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
    // task_config.time_updater = false;
    // task_config.btn_green = false;
    // task_config.btn_blue = false;
    // task_config.btn_yellow = false;
    task_config.neopixel = false;
    // task_config.display = false;
    task_config.dfplayer = false;
    // task_config.usb_power = false;
    // task_config.vsys_voltage = false;
    // task_config.alarm_settings = false;
    // task_config.minute_timer = false;

    // RTC
    // Initialize the RTC in a static cell, we will need it in multiple places
    static RTC: StaticCell<RefCell<Rtc<'static, peripherals::RTC>>> = StaticCell::new();
    let rtc_instance: Rtc<'static, peripherals::RTC> = Rtc::new(r.real_time_clock.rtc);
    let rtc_ref = RTC.init(RefCell::new(rtc_instance));

    // Orchestrate
    // there is no main loop, the tasks are spawned and run in parallel
    // orchestrating the tasks is done here:
    spawner.spawn(orchestrate(spawner)).unwrap();

    // Alarm settings
    if task_config.alarm_settings {
        spawner
            .spawn(manage_alarm_settings(spawner, r.flash))
            .unwrap();
    };

    // Power
    if task_config.usb_power {
        spawner.spawn(usb_power(spawner, r.vbus_power)).unwrap();
    };

    if task_config.vsys_voltage {
        spawner
            .spawn(vsys_voltage(spawner, r.vsys_resources))
            .unwrap();
    };

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
        spawner.spawn(display(spawner, r.display, rtc_ref)).unwrap();
    }

    // DFPlayer
    if task_config.dfplayer {
        spawner.spawn(sound(spawner, r.dfplayer)).unwrap();
    }

    // Minute timer
    if task_config.minute_timer {
        spawner.spawn(minute_timer(spawner, rtc_ref)).unwrap();
    }
}

/// This struct is used to configure which tasks are enabled
/// This is useful for troubleshooting, as we can disable tasks to reduce the binary size
/// and clutter in the output.
/// Also, we can disable tasks that are not needed for the current development stage and also test tasks in isolation.
/// For a production build we will need all tasks enabled
pub struct TaskConfig {
    pub btn_green: bool,
    pub btn_blue: bool,
    pub btn_yellow: bool,
    pub time_updater: bool,
    pub neopixel: bool,
    pub display: bool,
    pub dfplayer: bool,
    pub usb_power: bool,
    pub vsys_voltage: bool,
    pub alarm_settings: bool,
    pub minute_timer: bool,
}

impl Default for TaskConfig {
    fn default() -> Self {
        TaskConfig {
            btn_green: true,
            btn_blue: true,
            btn_yellow: true,
            time_updater: true,
            neopixel: true,
            display: true,
            dfplayer: true,
            usb_power: true,
            vsys_voltage: true,
            alarm_settings: true,
            minute_timer: true,
        }
    }
}

impl TaskConfig {
    pub fn new() -> Self {
        TaskConfig::default()
    }
}
