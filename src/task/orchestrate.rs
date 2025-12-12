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

        // Lock the mutex to get a mutable reference to the system state
        let mut system_state_guard = SYSTEM_STATE.lock().await;
        let Some(system_state) = system_state_guard.as_mut() else {
            warn!("System state not initialized");
            continue;
        };

        // react to the events
        handle_event(event, system_state).await;

        // Report successful event handling to watchdog
        report_task_success(TaskId::Orchestrator).await;

        drop(system_state_guard);
    }
}

/// Handles a single event by updating the system state and signaling appropriate tasks.
async fn handle_event(event: Event, system_state: &mut SystemState) {
    match event {
        Event::BlueBtn => {
            system_state.update_interaction_tick(Instant::now().as_millis());
            handle_blue_button_press(system_state).await;
            signal_display_update();
            handle_button_led_on_button_press(system_state);
        }
        Event::GreenBtn => {
            system_state.update_interaction_tick(Instant::now().as_millis());
            handle_green_button_press(system_state).await;
            signal_display_update();
            handle_button_led_on_button_press(system_state);
        }
        Event::YellowBtn => {
            system_state.update_interaction_tick(Instant::now().as_millis());
            handle_yellow_button_press(system_state).await;
            signal_display_update();
            handle_button_led_on_button_press(system_state);
        }
        Event::InteractiveModeTimeout => handle_interactive_mode_timeout_event(system_state).await,
        Event::Vbus(usb) => handle_vbus_event(system_state, usb),
        Event::Vsys(voltage) => handle_vsys_event(system_state, voltage),
        Event::AlarmSettingsReadFromFlash(alarm_settings) => {
            handle_alarm_settings_read_event(system_state, alarm_settings).await;
        }
        Event::SystemSettingsReadFromFlash(system_settings) => {
            handle_system_settings_read_event(system_state, system_settings);
        }
        Event::Scheduler((hour, minute, second)) => {
            handle_scheduler_event(system_state, hour, minute, second);
        }
        Event::RtcUpdated => handle_rtc_updated_event(system_state).await,
        Event::Alarm => handle_alarm_event(system_state),
        Event::AlarmStop => handle_alarm_stop_event(system_state),
        Event::SunriseEffectFinished => handle_sunrise_effect_finished_event(system_state),
    }
}

/// Handles interactive mode timeout by auto-saving changes and returning to normal mode
async fn handle_interactive_mode_timeout_event(system_state: &mut SystemState) {
    info!("Interactive mode timeout, returning to normal mode");
    // Auto-save any pending changes before returning to normal mode
    match system_state.operation_mode {
        OperationMode::SetVolume | OperationMode::SetClockBrightness => {
            handle_system_settings_update(system_state).await;
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
    signal_display_update();
}

/// Handles USB power state changes
fn handle_vbus_event(system_state: &mut SystemState, usb: bool) {
    info!("Vbus event, usb: {}", usb);
    system_state.power_state.set_usb_power(usb);
    if !system_state.power_state.get_usb_power() {
        signal_vsys_wake();
    }
    signal_display_update();
}

/// Handles system voltage changes
fn handle_vsys_event(system_state: &mut SystemState, voltage: f32) {
    info!("Vsys event, voltage: {}", voltage);
    system_state.power_state.set_vsys(voltage);
    system_state.power_state.set_battery_level();
    signal_display_update();
}

/// Handles alarm settings loaded from flash
async fn handle_alarm_settings_read_event(system_state: &mut SystemState, alarm_settings: AlarmSettings) {
    info!("Alarm time read from flash: {:?}", alarm_settings);
    system_state.alarm_settings = alarm_settings;
    let mut init_state = INIT_STATE.lock().await;
    init_state.mark_alarm_settings_loaded();
    if init_state.is_ready() {
        drop(init_state);
        handle_system_ready_event(system_state);
    }
}

/// Handles system settings loaded from flash
fn handle_system_settings_read_event(system_state: &mut SystemState, system_settings: SystemSettings) {
    info!("System settings read from flash: {:?}", system_settings);
    let volume = system_settings.get_volume();
    system_state.system_settings = system_settings;
    // Update runtime settings for sound and light tasks
    signal_sound_volume_update(volume);
    signal_neopixel_brightness_update();
}

/// Handles RTC update completion
async fn handle_rtc_updated_event(system_state: &mut SystemState) {
    info!("RTC updated event");
    let mut init_state = INIT_STATE.lock().await;
    init_state.mark_rtc_ready();
    if init_state.is_ready() {
        drop(init_state);
        handle_system_ready_event(system_state);
    }
    signal_display_update();
}

/// Handles system ready event (initialization complete)
fn handle_system_ready_event(system_state: &mut SystemState) {
    info!("System initialization complete");
    system_state.complete_initialization();
    signal_display_update();
    signal_scheduler_wake();
}

/// Handles the scheduler event which updates display and light effects.
fn handle_scheduler_event(system_state: &SystemState, hour: u8, minute: u8, second: u8) {
    // Don't update anything if we're still initializing
    if system_state.operation_mode == OperationMode::Initializing {
        return;
    }

    // update the light effects if the alarm is not enabled and the alarm state is None
    if system_state.alarm_state == AlarmState::None && !system_state.alarm_settings.get_enabled() {
        signal_lightfx_start(hour, minute, second);
    }
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

/// Handles system settings update by writing to flash and updating runtime settings.
async fn handle_system_settings_update(system_state: &SystemState) {
    send_flash_write_command(SettingsWriteCommand::SystemSettings(
        system_state.system_settings.clone(),
    ))
    .await;

    // Update runtime settings for sound and light tasks
    signal_sound_volume_update(system_state.system_settings.get_volume());
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
    signal_sound_stop();
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
fn handle_alarm_event(system_state: &mut SystemState) {
    info!("Alarm event");
    system_state.randomize_alarm_stop_button_sequence();
    system_state.set_alarm_mode();
    signal_display_update();
    signal_lightfx_start(0, 0, 0);
    signal_button_leds(ButtonLedCommand::On);
}

/// Handles the alarm stop event by transitioning back to normal mode.
fn handle_alarm_stop_event(system_state: &mut SystemState) {
    info!("Alarm stop event");
    if system_state.alarm_state.is_active() {
        system_state.set_normal_mode();
        signal_display_update();
        signal_lightfx_stop();
        signal_lightfx_start(0, 0, 0);
        signal_sound_stop();
        signal_button_leds(ButtonLedCommand::Off);
    }
}

/// Handles the sunrise effect finished event by transitioning to noise phase.
fn handle_sunrise_effect_finished_event(system_state: &mut SystemState) {
    info!("Sunrise effect finished event");
    system_state.set_alarm_state(AlarmState::Noise);
    signal_sound_start();
    signal_lightfx_start(0, 0, 0);
}

/// Handle state changes when the green button is pressed
async fn handle_green_button_press(system_state: &mut SystemState) {
    match system_state.operation_mode {
        OperationMode::Initializing => {
            // Ignore button presses during initialization
        }
        OperationMode::Normal => {
            system_state.toggle_alarm_enabled();
            handle_alarm_settings_update(system_state).await;
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
                handle_alarm_stop_event(system_state);
            }
        }
        OperationMode::Standby => {
            system_state.wake_up();
            handle_wakeup_event();
        }
    }
}

/// Handles button LED control when a button is pressed (non-alarm mode only)
fn handle_button_led_on_button_press(system_state: &SystemState) {
    // Only trigger the timeout if not in alarm mode
    // During alarm mode, button LEDs are controlled by alarm start/stop
    if system_state.operation_mode != OperationMode::Alarm {
        signal_button_leds(ButtonLedCommand::OnWithTimeout);
    }
}

/// Handle state changes when the blue button is pressed
async fn handle_blue_button_press(system_state: &mut SystemState) {
    match system_state.operation_mode {
        OperationMode::Initializing => {
            // Ignore button presses during initialization
        }
        OperationMode::Normal => {
            system_state.set_set_alarm_time_mode();
        }
        OperationMode::SetAlarmTime => {
            handle_alarm_settings_update(system_state).await;
            system_state.set_normal_mode();
        }
        OperationMode::Menu => {
            system_state.set_standby_mode();
            handle_standby_event();
        }
        OperationMode::SystemInfo => {
            system_state.set_normal_mode();
        }
        OperationMode::SettingsMenu => {
            system_state.set_clock_brightness_mode();
        }
        OperationMode::SetVolume => {
            // Save volume to flash and return to settings menu
            handle_system_settings_update(system_state).await;
            system_state.set_settings_menu_mode();
        }
        OperationMode::SetClockBrightness => {
            // Save brightness to flash and return to settings menu
            handle_system_settings_update(system_state).await;
            system_state.set_settings_menu_mode();
        }
        OperationMode::SetTimeManual => {
            // Set the RTC time and return to settings menu
            let (hour, minute) = system_state.manual_time_buffer;
            handle_manual_time_set(hour, minute).await;
            system_state.set_settings_menu_mode();
        }
        OperationMode::Alarm => {
            if system_state.alarm_settings.get_first_valid_stop_alarm_button() == Button::Blue {
                system_state.alarm_settings.erase_first_valid_stop_alarm_button();
            }
            if system_state.alarm_settings.is_alarm_stop_button_sequence_complete() {
                handle_alarm_stop_event(system_state);
            }
        }
        OperationMode::Standby => {
            system_state.wake_up();
            handle_wakeup_event();
        }
    }
}

/// Handle state changes when the yellow button is pressed
async fn handle_yellow_button_press(system_state: &mut SystemState) {
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
            // Get current time from RTC for manual time setting
            let (hour, minute) = rtc_get_time().await.map_or((0, 0), |dt| (dt.hour, dt.minute));
            system_state.set_time_manual_mode(hour, minute);
        }
        OperationMode::SetAlarmTime => {
            system_state.increment_alarm_minute();
        }
        OperationMode::SetVolume => {
            system_state.system_settings.decrement_volume();
        }
        OperationMode::SetClockBrightness => {
            system_state.system_settings.decrement_clock_brightness();
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
                handle_alarm_stop_event(system_state);
            }
        }
        OperationMode::Standby => {
            system_state.wake_up();
            handle_wakeup_event();
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
        let dt: DateTime = rtc_get_time().await.map_or_else(
            || {
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
            },
            |dt| dt,
        );

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
                Duration::from_secs(60)
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
