//! Task to orchestrate the state transitions of the system.
use crate::task::state::*;
use core::cell::RefCell;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::peripherals::RTC;
use embassy_rp::rtc::Rtc;

/// This task is responsible for the state transitions of the system. It acts as the main task of the system.
/// It receives events from the other tasks and reacts to them by changing the state of the system.
#[embassy_executor::task]
pub async fn orchestrate(_spawner: Spawner, rtc_ref: &'static RefCell<Rtc<'static, RTC>>) {
    // initialize the state manager and put it into the mutex
    {
        let state_manager = StateManager::new();
        *(STATE_MANAGER_MUTEX.lock().await) = Some(state_manager);
    }

    let event_receiver = EVENT_CHANNEL.receiver();
    let flash_sender = FLASH_CHANNEL.sender();

    info!("Orchestrate task started");

    // // just testing: set the alarm time to 7:30 and enable the alarm
    // state_manager.alarm_settings.enabled = true;
    // state_manager.alarm_settings.time = (7, 30);
    // flash_sender
    //     .send(Commands::AlarmSettingsWriteToFlash(
    //         state_manager.alarm_settings.clone(),
    //     ))
    //     .await;

    loop {
        // receive the events, halting the task until an event is received
        let event = event_receiver.receive().await;

        '_mutex_guard: {
            // Lock the mutex to get a mutable reference to the state manager
            let mut state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
            // Get a mutable reference to the state manager. We can unwrap here because we know that the state manager is initialized
            let state_manager = state_manager_guard.as_mut().unwrap();

            // react to the events
            match event {
                Events::BlueBtn(presses) => {
                    info!("Blue button pressed, presses: {}", presses);
                }
                Events::GreenBtn(presses) => {
                    state_manager.handle_green_button_press();
                }
                Events::YellowBtn(presses) => {
                    info!("Yellow button pressed, presses: {}", presses);
                }
                Events::Vbus(usb) => {
                    info!("Vbus event, usb: {}", usb);
                    state_manager.power_state.usb_power = usb;
                }
                Events::Vsys(voltage) => {
                    info!("Vsys event, voltage: {}", voltage);
                    state_manager.power_state.vsys = voltage;
                    state_manager.power_state.set_battery_level();
                }
                Events::AlarmSettingsReadFromFlash(alarm_settings) => {
                    info!("Alarm time read from flash: {:?}", alarm_settings);
                    state_manager.alarm_settings = alarm_settings;
                }
            }
        }

        // at this point we have altered the state of the system, we can now trigger actions based on the state
        // for now we will just log the state, in another task :-)
        if let Ok(dt) = rtc_ref.borrow_mut().now() {
            info!(
                "orhestrate loop: {}-{:02}-{:02} {}:{:02}:{:02}",
                dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second,
            );
        }

        match _spawner.spawn(info_task(_spawner)) {
            Ok(_) => info!("info_task spawned"),
            Err(_) => info!("info_task spawn failed"),
        }
        // ToDo: send the state to the display task. This will be straightforward, as we will design the display task to
        // receive the state and update the display accordingly.

        // ToDo: send the state to the sound task. This will be straightforward, as there is only one sound to play, the alarm sound.

        // ToDo: send the state to the neopixel task. This will need a little thinking, as the neopixel hs different effects to display
    }
}

/// Task to log the state of the system.
///
/// This task is responsible for logging the state of the system. It is triggered by the orchestrate task.
/// This is just a simple way to prove the Mutex is working as expected.
#[embassy_executor::task]
pub async fn info_task(_spawner: Spawner) {
    info!("Info task started");
    let mut state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
    match state_manager_guard.as_mut() {
        Some(state_manager) => {
            info!("{:?}", state_manager);
        }
        None => {
            info!("Info task started, but the state manager is not initialized yet");
        }
    }
}
