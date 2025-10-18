//! # Orchestrate Tasks
//! Task to orchestrate the state transitions of the system.
use crate::task::alarm_trigger::{ALARM_SCHEDULE_DISABLE_SIGNAL, ALARM_SCHEDULE_UPDATE_SIGNAL};
use crate::task::state::{AlarmState, STATE_MANAGER_MUTEX, StateManager};
use crate::task::task_messages::{
    ALARM_EXPIRER_SIGNAL, Commands, DISPLAY_SIGNAL, EVENT_CHANNEL, Events, FLASH_CHANNEL,
    LIGHTFX_SIGNAL, LIGHTFX_STOP_SIGNAL, SCHEDULER_START_SIGNAL, SCHEDULER_STOP_SIGNAL,
    SCHEDULER_WAKE_SIGNAL, SOUND_START_SIGNAL, SOUND_STOP_SIGNAL, TIME_UPDATER_RESUME_SIGNAL,
    TIME_UPDATER_SUSPEND_SIGNAL, VSYS_WAKE_SIGNAL,
};
use crate::task::time_updater::RTC_MUTEX;
use defmt::{Debug2Format, info, warn};
use embassy_futures::select::select;
use embassy_rp::rtc::{DateTime, DayOfWeek};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Sender;
use embassy_time::{Duration, Timer};

/// Type alias for the flash channel sender used to communicate with the flash task.
type FlashSender = Sender<'static, CriticalSectionRawMutex, Commands, 1>;

/// This task is responsible for the state transitions of the system. It acts as the main task of the system.
/// It receives events from the other tasks and reacts to them by changing the state of the system.
#[embassy_executor::task]
pub async fn orchestrator() {
    info!("Orchestrate task starting");
    // initialize the state manager and put it into the mutex
    {
        let state_manager = StateManager::new();
        *(STATE_MANAGER_MUTEX.lock().await) = Some(state_manager);
    }

    // init the receiver for the event channel, this is the line we are listening on
    let event_receiver = EVENT_CHANNEL.receiver();

    // init the sender for the flash channel
    let flash_sender = FLASH_CHANNEL.sender();

    loop {
        // receive the events, halting the task until an event is received
        let event = event_receiver.receive().await;

        // Lock the mutex to get a mutable reference to the state manager
        let mut state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
        let Some(state_manager) = state_manager_guard.as_mut() else {
            warn!("State manager not initialized");
            continue;
        };

        // react to the events
        handle_event(event, state_manager, &flash_sender).await;

        drop(state_manager_guard);
    }
}

/// Handles a single event by updating the state manager and signaling appropriate tasks.
async fn handle_event(event: Events, state_manager: &mut StateManager, flash_sender: &FlashSender) {
    match event {
        Events::BlueBtn => {
            state_manager.handle_blue_button_press().await;
            DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
        }
        Events::GreenBtn => {
            state_manager.handle_green_button_press().await;
            DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
        }
        Events::YellowBtn => {
            state_manager.handle_yellow_button_press().await;
            DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
        }
        Events::Vbus(usb) => {
            info!("Vbus event, usb: {}", usb);
            state_manager.power_state.set_usb_power(usb);
            if !state_manager.power_state.get_usb_power() {
                VSYS_WAKE_SIGNAL.signal(Commands::VsysWakeUp);
            }
            DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
        }
        Events::Vsys(voltage) => {
            info!("Vsys event, voltage: {}", voltage);
            state_manager.power_state.set_vsys(voltage);
            state_manager.power_state.set_battery_level();
            DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
        }
        Events::AlarmSettingsReadFromFlash(alarm_settings) => {
            info!("Alarm time read from flash: {:?}", alarm_settings);
            state_manager.alarm_settings = alarm_settings;
        }
        Events::Scheduler((hour, minute, second)) => {
            info!("Scheduler event");
            handle_scheduler_event(state_manager, hour, minute, second);
        }
        Events::RtcUpdated => {
            info!("RTC updated event");
            DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
        }
        Events::AlarmSettingsNeedUpdate => {
            info!("Alarm settings must be updated event");
            handle_alarm_settings_update(state_manager, flash_sender).await;
        }
        Events::Standby => {
            handle_standby_event();
        }
        Events::WakeUp => {
            handle_wakeup_event();
        }
        Events::Alarm => {
            handle_alarm_event(state_manager);
        }
        Events::AlarmStop => {
            handle_alarm_stop_event(state_manager);
        }
        Events::SunriseEffectFinished => {
            handle_sunrise_effect_finished_event(state_manager);
        }
    }
}

/// Handles the scheduler event which updates display and light effects.
fn handle_scheduler_event(state_manager: &StateManager, hour: u8, minute: u8, second: u8) {
    // update the light effects if the alarm is not enabled and the alarm state is None
    if state_manager.alarm_state == AlarmState::None && !state_manager.alarm_settings.get_enabled()
    {
        LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((hour, minute, second)));
    }
    // update the display
    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
}

/// Handles alarm settings update by writing to flash and coordinating with alarm task.
async fn handle_alarm_settings_update(state_manager: &StateManager, flash_sender: &FlashSender) {
    flash_sender
        .send(Commands::AlarmSettingsWriteToFlash(
            state_manager.alarm_settings.clone(),
        ))
        .await;

    if state_manager.alarm_settings.get_enabled() {
        // if the alarm is enabled, we must update the light effects and signal the alarm task to reschedule
        LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
        ALARM_SCHEDULE_UPDATE_SIGNAL.signal(Commands::AlarmSettingsWriteToFlash(
            state_manager.alarm_settings.clone(),
        ));
    } else {
        // if the alarm is disabled, we must signal the alarm task to disable and wake up the scheduler early
        ALARM_SCHEDULE_DISABLE_SIGNAL.signal(Commands::AlarmSettingsWriteToFlash(
            state_manager.alarm_settings.clone(),
        ));
        SCHEDULER_WAKE_SIGNAL.signal(Commands::SchedulerWakeUp);
    }
}

/// Handles the standby event by stopping scheduler and suspending time updater.
fn handle_standby_event() {
    info!("Standby event");
    SCHEDULER_STOP_SIGNAL.signal(Commands::SchedulerStop);
    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
    LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
    if SOUND_START_SIGNAL.signaled() {
        SOUND_STOP_SIGNAL.signal(Commands::SoundUpdate);
    }
    TIME_UPDATER_SUSPEND_SIGNAL.signal(Commands::TimeUpdaterSuspend);
}

/// Handles the wake up event by starting scheduler and resuming time updater.
fn handle_wakeup_event() {
    info!("Wake up event");
    SCHEDULER_START_SIGNAL.signal(Commands::SchedulerStart);
    VSYS_WAKE_SIGNAL.signal(Commands::VsysWakeUp);
    TIME_UPDATER_SUSPEND_SIGNAL.reset();
    TIME_UPDATER_RESUME_SIGNAL.signal(Commands::TimeUpdaterResume);
}

/// Handles the alarm event by initializing alarm mode and starting effects.
fn handle_alarm_event(state_manager: &mut StateManager) {
    info!("Alarm event");
    state_manager.randomize_alarm_stop_buttom_sequence();
    state_manager.set_alarm_mode();
    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
    LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
    ALARM_EXPIRER_SIGNAL.signal(Commands::AlarmExpiry);
}

/// Handles the alarm stop event by transitioning back to normal mode.
fn handle_alarm_stop_event(state_manager: &mut StateManager) {
    info!("Alarm stop event");
    if state_manager.alarm_state.is_active() {
        state_manager.set_normal_mode();
        DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
        LIGHTFX_STOP_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
        LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
        SOUND_STOP_SIGNAL.signal(Commands::SoundUpdate);
    }
}

/// Handles the sunrise effect finished event by transitioning to noise phase.
fn handle_sunrise_effect_finished_event(state_manager: &mut StateManager) {
    info!("Sunrise effect finished event");
    state_manager.set_alarm_state(AlarmState::Noise);
    SOUND_START_SIGNAL.signal(Commands::SoundUpdate);
    LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
}

/// This task handles scheduling periodic display and LED updates by sending events to the Event Channel.
/// Alarm scheduling and triggering is now handled by the dedicated `alarm_trigger_task`.
#[embassy_executor::task]
pub async fn scheduler() {
    info!("scheduler task started");
    'mainloop: loop {
        // see if we must halt the task, then wait for the start signal
        if SCHEDULER_STOP_SIGNAL.signaled() {
            SCHEDULER_STOP_SIGNAL.reset();
            SCHEDULER_START_SIGNAL.wait().await;
        }

        // get the current time
        let dt: DateTime;
        '_rtc_mutex: {
            let rtc_guard = RTC_MUTEX.lock().await;
            let Some(rtc) = rtc_guard.as_ref() else {
                warn!("RTC not initialized");
                drop(rtc_guard);
                Timer::after(Duration::from_secs(3)).await;
                continue 'mainloop;
            };
            dt = match rtc.now() {
                Ok(dt) => dt,
                Err(e) => {
                    info!("RTC not running: {:?}", Debug2Format(&e));
                    // Return an empty DateTime
                    DateTime {
                        year: 0,
                        month: 0,
                        day: 0,
                        day_of_week: DayOfWeek::Monday,
                        hour: 0,
                        minute: 0,
                        second: 0,
                    }
                }
            };
        };

        EVENT_CHANNEL
            .sender()
            .send(Events::Scheduler((dt.hour, dt.minute, dt.second)))
            .await;

        // get the alarm enabled state to determine update frequency
        let alarm_enabled: bool;
        '_state_manager_mutex: {
            let state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
            let Some(state_manager) = state_manager_guard.as_ref() else {
                warn!("State manager not initialized");
                drop(state_manager_guard);
                Timer::after(Duration::from_secs(1)).await;
                continue 'mainloop;
            };
            alarm_enabled = state_manager.alarm_settings.get_enabled();
        }

        // calculate the downtime we need to wait until the next iteration
        let downtime: Duration = if alarm_enabled {
            // When alarm is enabled, we can wait longer since the RTC will handle the alarm
            Duration::from_secs(60)
        } else {
            // if the alarm is not enabled, we will be using the neopixel analog clock effect, which will need to be updated often
            // so we must wait for 3.75 seconds (60s / 16leds -> 3.75s until we must update the leds). To avoid visual glitches, we reduce that time by 10ms
            Duration::from_millis(3740)
        };

        // we either wait for the downtime or until we are woken up early. Whatever comes first, starts the next iteration.
        let downtime_timer = Timer::after(downtime);
        select(downtime_timer, SCHEDULER_WAKE_SIGNAL.wait()).await;
    }
}

/// This task handles the expiration of the alarm after 5 minutes.
#[embassy_executor::task]
pub async fn alarm_expirer() {
    info!("Alarm expirer task started");
    '_mainloop: loop {
        // wait for the alarm expiry watcher signal
        ALARM_EXPIRER_SIGNAL.wait().await;
        // wait for 5 minutes
        Timer::after(Duration::from_secs(300)).await;
        // send the alarm stop event
        EVENT_CHANNEL.sender().send(Events::AlarmStop).await;
    }
}
