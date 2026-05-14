//! # Orchestrate Tasks
//! Task to orchestrate the state transitions of the system.
use defmt::{info, warn};
use embassy_futures::select::select;
use embassy_rp::rtc::{DateTime, DayOfWeek};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Instant, Ticker, Timer};

use crate::{
    event::{Event, receive_event, send_event},
    state::{AlarmSettings, AlarmState, OperationMode, SYSTEM_STATE, SystemSettings, SystemState},
    task::{
        alarm_settings::{SettingsWriteCommand, send_flash_write_command},
        alarm_trigger::{signal_alarm_schedule_disable, signal_alarm_schedule_update},
        button_leds::{ButtonLedCommand, signal_button_leds},
        buttons::Button,
        display::signal_display_update,
        light_effects::{signal_lightfx_start, signal_lightfx_stop, signal_neopixel_brightness_update},
        power::signal_vsys_wake,
        rtc_manager::{rtc_get_time, rtc_set_time_manual},
        sound::{signal_sound_start, signal_sound_stop, signal_sound_volume_update},
        time_updater::{signal_time_updater_resume, signal_time_updater_suspend},
        watchdog::{TaskId, report_task_success},
    },
};

/// Tracks initialization state protected by a mutex
static INIT_STATE: Mutex<CriticalSectionRawMutex, InitializationState> = Mutex::new(InitializationState::new());

/// Struct to track what has been initialized
struct InitializationState {
    /// Whether the RTC has been set with valid time
    rtc_ready: bool,
    /// Whether alarm settings have been loaded from flash
    alarm_settings_loaded: bool,
}

impl InitializationState {
    /// Create a new initialization state with nothing ready
    const fn new() -> Self {
        Self {
            rtc_ready: false,
            alarm_settings_loaded: false,
        }
    }

    /// Check if both RTC and alarm settings are ready
    const fn is_ready(&self) -> bool {
        self.rtc_ready && self.alarm_settings_loaded
    }

    /// Mark RTC as ready
    const fn mark_rtc_ready(&mut self) {
        self.rtc_ready = true;
    }

    /// Mark alarm settings as loaded
    const fn mark_alarm_settings_loaded(&mut self) {
        self.alarm_settings_loaded = true;
    }
}

/// Update interval for the analog clock effect.
/// With 16 LEDs and 60 seconds, each LED represents ~3.75 seconds.
/// We tick every second to ensure smooth LED transitions without aliasing
/// between the ticker and the RTC's second updates.
const ANALOG_CLOCK_UPDATE_INTERVAL: Duration = Duration::from_millis(1000);

/// Signal for stopping the scheduler
static SCHEDULER_STOP_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal for starting the scheduler
static SCHEDULER_START_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal for waking the scheduler early
static SCHEDULER_WAKE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signals the scheduler to stop
pub fn signal_scheduler_stop() {
    SCHEDULER_STOP_SIGNAL.signal(());
}

/// Signals the scheduler to start
pub fn signal_scheduler_start() {
    SCHEDULER_START_SIGNAL.signal(());
}

/// Signals the scheduler to wake up early
pub fn signal_scheduler_wake() {
    SCHEDULER_WAKE_SIGNAL.signal(());
}

/// This task is responsible for the state transitions of the system. It acts as the main task of the system.
/// It receives events from the other tasks and reacts to them by changing the state of the system.
#[embassy_executor::task]
pub async fn orchestrator() {
    info!("Orchestrate task starting");
    // initialize the system state and put it into the mutex
    {
        let system_state = SystemState::new();
        *(SYSTEM_STATE.lock().await) = Some(system_state);
    }

    // Signal display to show initializing screen
    signal_display_update();

    loop {
        // receive the events, halting the task until an event is received
        let event = receive_event().await;

        // react to the events
        handle_event(event).await;

        // Report successful event handling to watchdog
        report_task_success(TaskId::Orchestrator).await;
    }
}

/// Handles a single event by updating the system state and signaling appropriate tasks.
async fn handle_event(event: Event) {
    let now_tick = Instant::now().as_millis();

    match event {
        Event::BlueBtn => {
            if handle_display_wake_if_off(now_tick).await {
                return;
            }
            handle_blue_button_press(now_tick).await;
            signal_display_update();
            handle_button_led_on_button_press().await;
        }
        Event::GreenBtn => {
            if handle_display_wake_if_off(now_tick).await {
                return;
            }
            handle_green_button_press(now_tick).await;
            signal_display_update();
            handle_button_led_on_button_press().await;
        }
        Event::YellowBtn => {
            if handle_display_wake_if_off(now_tick).await {
                return;
            }
            // system_state.update_interaction_tick(now_tick);
            handle_yellow_button_press(now_tick).await;
            signal_display_update();
            handle_button_led_on_button_press().await;
        }
        Event::InteractiveModeTimeout => handle_interactive_mode_timeout_event().await,
        Event::DisplaySleepTimeout => handle_display_sleep_timeout_event(now_tick).await,
        Event::Vbus(usb) => handle_vbus_event(usb).await,
        Event::Vsys(voltage) => handle_vsys_event(voltage).await,
        Event::AlarmSettingsReadFromFlash(alarm_settings) => {
            handle_alarm_settings_read_event(alarm_settings).await;
        }
        Event::SystemSettingsReadFromFlash(system_settings) => {
            handle_system_settings_read_event(system_settings).await;
        }
        Event::Scheduler((hour, minute, second)) => {
            handle_scheduler_event(hour, minute, second).await;
        }
        Event::RtcUpdated => handle_rtc_updated_event().await,
        Event::Alarm => handle_alarm_event().await,
        Event::AlarmStop => handle_alarm_stop_event(now_tick).await,
        Event::SunriseEffectFinished => handle_sunrise_effect_finished_event().await,
    }
}

/// Handles interactive mode timeout by auto-saving changes and returning to normal mode
async fn handle_interactive_mode_timeout_event() {
    info!("Interactive mode timeout, returning to normal mode");

    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    // Auto-save any pending changes before returning to normal mode
    match system_state.operation_mode {
        OperationMode::SetVolume | OperationMode::SetClockBrightness => {
            handle_system_settings_update().await;
        }
        OperationMode::SetTimeManual => {
            let (hour, minute) = system_state.manual_time_buffer;
            handle_manual_time_set(hour, minute).await;
        }
        OperationMode::SetAlarmTime => {
            handle_alarm_settings_update(system_state).await;
        }
        _ => {
            // No settings to save for Menu, SettingsMenu, SystemInfo
        }
    }

    system_state.set_normal_mode();
    drop(system_state_guard);

    signal_display_update();
}

/// Handles waking the display if it is currently off.
/// Returns true if a button press should be consumed for wake-up only.
async fn handle_display_wake_if_off(now_tick: u64) -> bool {
    let display_was_woken = {
        let mut system_state_guard = SYSTEM_STATE.lock().await;
        let Some(system_state) = system_state_guard.as_mut() else {
            warn!("System state not initialized");
            drop(system_state_guard);
            return false;
        };

        if system_state.is_display_off() {
            system_state.set_display_on();
            system_state.update_interaction_tick(now_tick);
            drop(system_state_guard);
            true
        } else {
            false
        }
    };

    if display_was_woken {
        signal_display_update();
    }

    display_was_woken
}

/// Handles the alarm-enabled display sleep timeout
async fn handle_display_sleep_timeout_event(now_tick: u64) {
    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    if system_state.should_alarm_display_sleep(now_tick) {
        system_state.set_display_off();
        drop(system_state_guard);
        signal_display_update();
    }
}

/// Handles USB power state changes
async fn handle_vbus_event(usb: bool) {
    info!("Vbus event, usb: {}", usb);
    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    system_state.power_state.set_usb_power(usb);
    if !system_state.power_state.get_usb_power() {
        signal_vsys_wake();
    }

    drop(system_state_guard);
    signal_display_update();
}

/// Handles system voltage changes with hysteresis to filter noise
/// Only updates if voltage change is significant (>0.1V) to avoid display flicker from noise
async fn handle_vsys_event(voltage: f32) {
    const VOLTAGE_HYSTERESIS: f32 = 0.1; // Minimum voltage change to trigger update

    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    let current_voltage = system_state.power_state.get_vsys();
    let voltage_delta = (voltage - current_voltage).abs();

    // Only update if this is the first reading (current is 0.0) or change is significant
    if current_voltage == 0.0 || voltage_delta > VOLTAGE_HYSTERESIS {
        info!("Vsys event, voltage: {}V (delta: {}V)", voltage, voltage_delta);

        system_state.power_state.set_vsys(voltage);
        system_state.power_state.set_battery_level();

        drop(system_state_guard);

        signal_display_update();
    } else {
        info!(
            "Vsys event ignored (within hysteresis): {}V (delta: {}V)",
            voltage, voltage_delta
        );
    }
}

/// Handles alarm settings loaded from flash
async fn handle_alarm_settings_read_event(alarm_settings: AlarmSettings) {
    info!("Alarm time read from flash: {:?}", alarm_settings);

    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    system_state.alarm_settings = alarm_settings;
    drop(system_state_guard);

    let mut init_state = INIT_STATE.lock().await;
    init_state.mark_alarm_settings_loaded();
    if init_state.is_ready() {
        drop(init_state);
        handle_system_ready_event().await;
    }
}

/// Handles system settings loaded from flash
async fn handle_system_settings_read_event(system_settings: SystemSettings) {
    info!("System settings read from flash: {:?}", system_settings);

    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    let volume = system_settings.get_volume();

    system_state.system_settings = system_settings;
    drop(system_state_guard);

    // Update runtime settings for sound and light tasks
    signal_sound_volume_update(volume);
    signal_neopixel_brightness_update();
}

/// Handles RTC update completion
async fn handle_rtc_updated_event() {
    info!("RTC updated event");
    let mut init_state = INIT_STATE.lock().await;
    init_state.mark_rtc_ready();
    if init_state.is_ready() {
        drop(init_state);
        handle_system_ready_event().await;
    }
    signal_display_update();
}

/// Handles system ready event (initialization complete)
async fn handle_system_ready_event() {
    info!("System initialization complete");
    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    system_state.complete_initialization();
    drop(system_state_guard);

    signal_display_update();
    signal_scheduler_wake();
}

/// Handles the scheduler event which updates display and light effects.
async fn handle_scheduler_event(hour: u8, minute: u8, second: u8) {
    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    // Don't update anything if we're still initializing
    if system_state.operation_mode == OperationMode::Initializing {
        drop(system_state_guard);
        return;
    }

    // update the light effects if the alarm is not enabled and the alarm state is None
    if system_state.alarm_state == AlarmState::None && !system_state.alarm_settings.get_enabled() {
        signal_lightfx_start(hour, minute, second);
    }

    drop(system_state_guard);

    // update the display
    signal_display_update();
}

/// Handles alarm settings update by writing to flash and coordinating with alarm task.
async fn handle_alarm_settings_update(system_state: &SystemState) {
    send_flash_write_command(SettingsWriteCommand::AlarmSettings(system_state.alarm_settings.clone())).await;

    if system_state.alarm_settings.get_enabled() {
        // if the alarm is enabled, we must update the light effects and signal the alarm task to reschedule
        signal_lightfx_start(0, 0, 0);
        signal_alarm_schedule_update();
    } else {
        // if the alarm is disabled, we must signal the alarm task to disable and wake up the scheduler early
        signal_alarm_schedule_disable();
        signal_scheduler_wake();
    }
}

/// Handles alarm settings update by writing to flash and coordinating with alarm task.
async fn handle_alarm_settings_update_with_snapshot(alarm_settings: AlarmSettings) {
    send_flash_write_command(SettingsWriteCommand::AlarmSettings(alarm_settings.clone())).await;

    if alarm_settings.get_enabled() {
        // if the alarm is enabled, we must update the light effects and signal the alarm task to reschedule
        signal_lightfx_start(0, 0, 0);
        signal_alarm_schedule_update();
    } else {
        // if the alarm is disabled, we must signal the alarm task to disable and wake up the scheduler early
        signal_alarm_schedule_disable();
        signal_scheduler_wake();
    }
}

/// Handles system settings update by writing to flash and updating runtime settings.
async fn handle_system_settings_update() {
    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    let system_settings = system_state.system_settings;
    drop(system_state_guard);

    send_flash_write_command(SettingsWriteCommand::SystemSettings(system_settings)).await;

    // Update runtime settings for sound and light tasks
    signal_sound_volume_update(system_settings.get_volume());
    signal_neopixel_brightness_update();
}

/// Handles manual time set by updating the RTC.
async fn handle_manual_time_set(hour: u8, minute: u8) {
    rtc_set_time_manual(hour, minute).await;
    signal_display_update();
}

/// Handles the standby event by stopping scheduler and suspending time updater.
fn handle_standby_event() {
    info!("Standby event");
    signal_scheduler_stop();
    signal_display_update();
    signal_lightfx_start(0, 0, 0);
    signal_time_updater_suspend();
}

/// Handles the wake up event by starting scheduler and resuming time updater.
fn handle_wakeup_event() {
    info!("Wake up event");
    signal_scheduler_start();
    signal_vsys_wake();
    signal_time_updater_resume();
}

/// Handles the alarm event by initializing alarm mode and starting effects.
async fn handle_alarm_event() {
    info!("Alarm event");

    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    system_state.randomize_alarm_stop_button_sequence();
    system_state.set_alarm_mode();
    system_state.set_display_on();
    drop(system_state_guard);

    signal_display_update();
    signal_lightfx_start(0, 0, 0);
    signal_button_leds(ButtonLedCommand::On);
}

/// Handles the alarm stop event by transitioning back to normal mode.
async fn handle_alarm_stop_event(now_tick: u64) {
    info!("Alarm stop event");

    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    if system_state.alarm_state.is_active() {
        system_state.set_normal_mode();
        system_state.set_display_on();
        system_state.update_interaction_tick(now_tick);
        drop(system_state_guard);

        signal_display_update();
        signal_lightfx_stop();
        signal_lightfx_start(0, 0, 0);
        signal_sound_stop();
        signal_button_leds(ButtonLedCommand::Off);
    }
}

/// Handles the sunrise effect finished event by transitioning to noise phase.
async fn handle_sunrise_effect_finished_event() {
    info!("Sunrise effect finished event");

    let mut system_state_guard = SYSTEM_STATE.lock().await;
    let Some(system_state) = system_state_guard.as_mut() else {
        warn!("System state not initialized");
        drop(system_state_guard);
        return;
    };

    system_state.set_alarm_state(AlarmState::Noise);
    drop(system_state_guard);

    signal_sound_start();
    signal_lightfx_start(0, 0, 0);
}

/// Handles button LED control when a button is pressed (non-alarm mode only)
async fn handle_button_led_on_button_press() {
    // Only trigger the timeout if not in alarm mode
    // During alarm mode, button LEDs are controlled by alarm start/stop
    let not_in_operation_alarm = {
        let mut system_state_guard = SYSTEM_STATE.lock().await;
        let Some(system_state) = system_state_guard.as_mut() else {
            warn!("System state not initialized");
            drop(system_state_guard);
            return;
        };

        system_state.operation_mode != OperationMode::Alarm
    };

    if not_in_operation_alarm {
        signal_button_leds(ButtonLedCommand::OnWithTimeout);
    }
}

/// Handle state changes when the green button is pressed
async fn handle_green_button_press(now_tick: u64) {
    enum GreenPostAction {
        None,
        PersistAlarmSettings(AlarmSettings),
        AlarmStop(u64),
        Wakeup,
    }

    let mut action = GreenPostAction::None;

    {
        let mut system_state_guard = SYSTEM_STATE.lock().await;
        let Some(system_state) = system_state_guard.as_mut() else {
            warn!("System state not initialized");
            drop(system_state_guard);
            return;
        };

        system_state.update_interaction_tick(now_tick);

        match system_state.operation_mode {
            OperationMode::Initializing => {
                // Ignore button presses during initialization
            }
            OperationMode::Normal => {
                system_state.toggle_alarm_enabled();
                action = GreenPostAction::PersistAlarmSettings(system_state.alarm_settings.clone());
            }
            OperationMode::SetAlarmTime => {
                system_state.increment_alarm_hour();
            }
            OperationMode::Menu => {
                system_state.set_settings_menu_mode();
            }
            OperationMode::SystemInfo => {
                // Advance to next page, or exit if on last page
                if !system_state.next_system_info_page() {
                    system_state.set_normal_mode();
                }
            }
            OperationMode::SettingsMenu => {
                system_state.set_volume_mode();
            }
            OperationMode::SetVolume => {
                system_state.system_settings.increment_volume();
            }
            OperationMode::SetClockBrightness => {
                system_state.system_settings.increment_clock_brightness();
                // Live preview: update NeoPixel brightness immediately
                signal_neopixel_brightness_update();
            }
            OperationMode::SetTimeManual => {
                system_state.increment_manual_hour();
            }
            OperationMode::Alarm => {
                if system_state.alarm_settings.get_first_valid_stop_alarm_button() == Button::Green {
                    system_state.alarm_settings.erase_first_valid_stop_alarm_button();
                }
                if system_state.alarm_settings.is_alarm_stop_button_sequence_complete() {
                    action = GreenPostAction::AlarmStop(now_tick);
                }
            }
            OperationMode::Standby => {
                system_state.wake_up();
                action = GreenPostAction::Wakeup;
            }
        }
    } // lock dropped here

    match action {
        GreenPostAction::None => {}
        GreenPostAction::Wakeup => {
            handle_wakeup_event();
        }
        GreenPostAction::AlarmStop(tick) => {
            handle_alarm_stop_event(tick).await;
        }
        GreenPostAction::PersistAlarmSettings(alarm_settings) => {
            handle_alarm_settings_update_with_snapshot(alarm_settings).await;
        }
    }
}

/// Handle state changes when the blue button is pressed
async fn handle_blue_button_press(now_tick: u64) {
    enum BluePostAction {
        None,
        PersistAlarmSettingsAndSetNormal(AlarmSettings),
        PersistSystemSettingsAndSetSettingsMenu,
        SetManualTimeAndSetSettingsMenu(u8, u8),
        AlarmStop(u64),
        EnterStandby,
        Wakeup,
    }

    let mut action = BluePostAction::None;

    {
        let mut system_state_guard = SYSTEM_STATE.lock().await;
        let Some(system_state) = system_state_guard.as_mut() else {
            warn!("System state not initialized");
            drop(system_state_guard);
            return;
        };

        system_state.update_interaction_tick(now_tick);

        match system_state.operation_mode {
            OperationMode::Initializing => {
                // Ignore button presses during initialization
            }
            OperationMode::Normal => {
                system_state.set_set_alarm_time_mode();
            }
            OperationMode::SetAlarmTime => {
                // Persist current alarm settings after dropping lock
                action = BluePostAction::PersistAlarmSettingsAndSetNormal(system_state.alarm_settings.clone());
            }
            OperationMode::Menu => {
                system_state.set_standby_mode();
                action = BluePostAction::EnterStandby;
            }
            OperationMode::SystemInfo => {
                system_state.set_normal_mode();
            }
            OperationMode::SettingsMenu => {
                system_state.set_clock_brightness_mode();
            }
            OperationMode::SetVolume | OperationMode::SetClockBrightness => {
                // Save settings to flash after dropping lock, then return to settings menu
                action = BluePostAction::PersistSystemSettingsAndSetSettingsMenu;
            }
            OperationMode::SetTimeManual => {
                let (hour, minute) = system_state.manual_time_buffer;
                // Set RTC after dropping lock, then return to settings menu
                action = BluePostAction::SetManualTimeAndSetSettingsMenu(hour, minute);
            }
            OperationMode::Alarm => {
                if system_state.alarm_settings.get_first_valid_stop_alarm_button() == Button::Blue {
                    system_state.alarm_settings.erase_first_valid_stop_alarm_button();
                }
                if system_state.alarm_settings.is_alarm_stop_button_sequence_complete() {
                    action = BluePostAction::AlarmStop(now_tick);
                }
            }
            OperationMode::Standby => {
                system_state.wake_up();
                action = BluePostAction::Wakeup;
            }
        }
    } // lock dropped here

    match action {
        BluePostAction::None => {}
        BluePostAction::EnterStandby => {
            handle_standby_event();
        }
        BluePostAction::Wakeup => {
            handle_wakeup_event();
        }
        BluePostAction::AlarmStop(tick) => {
            handle_alarm_stop_event(tick).await;
        }
        BluePostAction::PersistAlarmSettingsAndSetNormal(alarm_settings) => {
            handle_alarm_settings_update_with_snapshot(alarm_settings).await;

            let mut system_state_guard = SYSTEM_STATE.lock().await;
            if let Some(system_state) = system_state_guard.as_mut() {
                system_state.set_normal_mode();
            } else {
                warn!("System state not initialized");
            }
        }
        BluePostAction::PersistSystemSettingsAndSetSettingsMenu => {
            handle_system_settings_update().await;

            let mut system_state_guard = SYSTEM_STATE.lock().await;
            if let Some(system_state) = system_state_guard.as_mut() {
                system_state.set_settings_menu_mode();
            } else {
                warn!("System state not initialized");
            }
        }
        BluePostAction::SetManualTimeAndSetSettingsMenu(hour, minute) => {
            handle_manual_time_set(hour, minute).await;

            let mut system_state_guard = SYSTEM_STATE.lock().await;
            if let Some(system_state) = system_state_guard.as_mut() {
                system_state.set_settings_menu_mode();
            } else {
                warn!("System state not initialized");
            }
        }
    }
}

/// Handle state changes when the yellow button is pressed
async fn handle_yellow_button_press(now_tick: u64) {
    enum YellowPostAction {
        None,
        FetchRtcAndSetManualMode,
        AlarmStop(u64),
        Wakeup,
    }

    let mut action = YellowPostAction::None;

    {
        let mut system_state_guard = SYSTEM_STATE.lock().await;
        let Some(system_state) = system_state_guard.as_mut() else {
            warn!("System state not initialized");
            return;
        };

        system_state.update_interaction_tick(now_tick);

        match system_state.operation_mode {
            OperationMode::Initializing => {
                // Ignore button presses during initialization
            }
            OperationMode::Normal => {
                system_state.set_menu_mode();
            }
            OperationMode::Menu => {
                system_state.set_system_info_mode();
            }
            OperationMode::SystemInfo => {
                system_state.set_normal_mode();
            }
            OperationMode::SettingsMenu => {
                // Need RTC async read; do it after dropping lock
                action = YellowPostAction::FetchRtcAndSetManualMode;
            }
            OperationMode::SetAlarmTime => {
                system_state.increment_alarm_minute();
            }
            OperationMode::SetVolume => {
                system_state.system_settings.decrement_volume();
            }
            OperationMode::SetClockBrightness => {
                system_state.system_settings.decrement_clock_brightness();
                drop(system_state_guard);
                // Live preview: update NeoPixel brightness immediately
                signal_neopixel_brightness_update();
            }
            OperationMode::SetTimeManual => {
                system_state.increment_manual_minute();
            }
            OperationMode::Alarm => {
                if system_state.alarm_settings.get_first_valid_stop_alarm_button() == Button::Yellow {
                    system_state.alarm_settings.erase_first_valid_stop_alarm_button();
                }
                if system_state.alarm_settings.is_alarm_stop_button_sequence_complete() {
                    // Need async stop handling after dropping lock
                    action = YellowPostAction::AlarmStop(now_tick);
                }
            }
            OperationMode::Standby => {
                system_state.wake_up();
                action = YellowPostAction::Wakeup;
            }
        }
    } // lock dropped here

    match action {
        YellowPostAction::None => {}
        YellowPostAction::Wakeup => {
            handle_wakeup_event();
        }
        YellowPostAction::AlarmStop(tick) => {
            handle_alarm_stop_event(tick).await;
        }
        YellowPostAction::FetchRtcAndSetManualMode => {
            let (hour, minute) = rtc_get_time().await.map_or((0, 0), |dt| (dt.hour, dt.minute));

            let mut system_state_guard = SYSTEM_STATE.lock().await;
            let Some(system_state) = system_state_guard.as_mut() else {
                warn!("System state not initialized");
                drop(system_state_guard);
                return;
            };

            system_state.set_time_manual_mode(hour, minute);
        }
    }
}

/// This task handles scheduling periodic display and LED updates by sending events to the Event Channel.
/// Alarm scheduling and triggering is now handled by the dedicated `alarm_trigger_task`.
/// Also handles menu timeout checking.
#[embassy_executor::task]
pub async fn scheduler() {
    info!("scheduler task started");
    // Start with a ticker for the default update rate when alarm is disabled
    let mut ticker = Ticker::every(ANALOG_CLOCK_UPDATE_INTERVAL);
    let mut last_alarm_enabled_state: Option<bool> = None;
    let mut last_menu_check = Instant::now();

    'mainloop: loop {
        // see if we must halt the task, then wait for the start signal
        if SCHEDULER_STOP_SIGNAL.signaled() {
            SCHEDULER_STOP_SIGNAL.reset();
            SCHEDULER_START_SIGNAL.wait().await;
        }

        // Get the current time from RTC manager
        let dt: DateTime = rtc_get_time().await.unwrap_or_else(|| {
            info!("RTC not running");
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
        });

        send_event(Event::Scheduler((dt.hour, dt.minute, dt.second))).await;

        // Report successful scheduler iteration to watchdog
        report_task_success(TaskId::Orchestrator).await;

        // Check for menu timeout every second
        if Instant::now().duration_since(last_menu_check) >= Duration::from_secs(1) {
            last_menu_check = Instant::now();

            // Check if we need to timeout any interactive mode
            let current_tick = Instant::now().as_millis();

            let should_timeout: bool = {
                let system_state_guard = SYSTEM_STATE.lock().await;
                system_state_guard
                    .as_ref()
                    .is_some_and(|state| state.should_interactive_mode_timeout(current_tick))
            };

            if should_timeout {
                send_event(Event::InteractiveModeTimeout).await;
            }

            let should_sleep: bool = {
                let system_state_guard = SYSTEM_STATE.lock().await;
                system_state_guard
                    .as_ref()
                    .is_some_and(|state| state.should_alarm_display_sleep(current_tick))
            };

            if should_sleep {
                send_event(Event::DisplaySleepTimeout).await;
            }
        }

        // get the alarm enabled state to determine update frequency
        let alarm_enabled: bool;
        '_system_state_mutex: {
            let system_state_guard = SYSTEM_STATE.lock().await;
            let Some(system_state) = system_state_guard.as_ref() else {
                warn!("System state not initialized");
                drop(system_state_guard);
                Timer::after(Duration::from_secs(1)).await;
                continue 'mainloop;
            };
            alarm_enabled = system_state.alarm_settings.get_enabled();
        }

        // Check if the alarm enabled state changed and recreate ticker if needed
        if last_alarm_enabled_state != Some(alarm_enabled) {
            let update_period = if alarm_enabled {
                // When alarm is enabled, we can wait longer since the RTC will handle the alarm
                Duration::from_secs(10)
            } else {
                // if the alarm is not enabled, we will be using the neopixel analog clock effect
                // tick every second to ensure smooth LED transitions
                ANALOG_CLOCK_UPDATE_INTERVAL
            };
            ticker = Ticker::every(update_period);
            last_alarm_enabled_state = Some(alarm_enabled);
        }

        // Wait for either the next tick or an early wake-up signal, whichever comes first
        select(ticker.next(), SCHEDULER_WAKE_SIGNAL.wait()).await;
    }
}
