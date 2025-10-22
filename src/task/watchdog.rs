//! Watchdog task to reset the system if it stops being fed
//!
//! This module provides a custom watchdog implementation that monitors the health
//! of critical system tasks. It uses a countdown timer approach rather than
//! constantly feeding a hardware watchdog, which provides more controlled reset behavior.
//!
//! The watchdog will trigger a system reset if:
//! - Critical tasks don't report success within the countdown period
//! - The countdown timer expires without all tasks being healthy

use defmt::{Format, info, warn};
use embassy_rp::{Peri, peripherals::WATCHDOG, watchdog::Watchdog};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Duration, Instant, Timer};

/// How long our custom countdown timer runs before triggering a reset (15 minutes)
const COUNTDOWN_TIMEOUT: Duration = Duration::from_secs(900);
/// How often we check task health and update our countdown
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(60);
/// Hardware watchdog timeout (short, used only for actual reset)
const HARDWARE_WATCHDOG_TIMEOUT: Duration = Duration::from_millis(8000);

/// Task identifiers for health tracking
#[derive(Debug, Clone, Copy, Eq, PartialEq, Format)]
pub enum TaskId {
    /// Orchestrator task (state machine coordination) - MUST report regularly
    Orchestrator,
    /// Display handler task - MUST report regularly
    Display,
    /// Alarm trigger task - MUST report regularly (even when waiting)
    AlarmTrigger,
    /// Time updater task (WiFi/RTC synchronization) - Critical, runs after startup
    TimeUpdater,
}

impl TaskId {
    /// Returns the maximum time allowed between health reports for this task
    const fn max_report_interval(self) -> Duration {
        match self {
            Self::Orchestrator | Self::Display => Duration::from_secs(120), // 2 minutes
            Self::AlarmTrigger => Duration::from_secs(300),                 // 5 minutes
            Self::TimeUpdater => Duration::from_secs(25200),                // 7 hours (refreshes every 6h)
        }
    }
}

/// Task health tracking with last-seen timestamp
#[derive(Copy, Clone, Format, Debug)]
struct TaskHealth {
    /// When this task last reported success
    last_report: Option<Instant>,
    /// Whether this task has ever reported
    has_reported: bool,
}

impl TaskHealth {
    /// Create a new `TaskHealth` instance
    const fn new() -> Self {
        Self {
            last_report: None,
            has_reported: false,
        }
    }

    /// Check if this task is healthy based on its max report interval
    fn is_healthy(&self, max_interval: Duration) -> bool {
        self.last_report
            .is_some_and(|last| Instant::now().duration_since(last) < max_interval)
    }
}

/// System health state with custom countdown timer
struct SystemHealth {
    /// Health status of each task
    /// Order: Orchestrator, Display, `AlarmTrigger`, `TimeUpdater`
    tasks: [TaskHealth; 4],
    /// When the system was initialized (for startup grace period)
    startup_time: Instant,
    /// Countdown timer - when this expires, we trigger hardware watchdog reset
    countdown_deadline: Option<Instant>,
}

impl SystemHealth {
    /// Create a new `SystemHealth` instance
    const fn new() -> Self {
        Self {
            tasks: [TaskHealth::new(); 4],
            startup_time: Instant::MIN,
            countdown_deadline: None,
        }
    }

    /// Initialize the startup time (called on first health check)
    fn init_if_needed(&mut self) {
        if self.startup_time == Instant::MIN {
            self.startup_time = Instant::now();
        }
    }

    /// Report a task as succeeded
    fn set_task_succeeded(&mut self, task_id: TaskId) {
        let index = task_id as usize;
        self.tasks[index].last_report = Some(Instant::now());
        self.tasks[index].has_reported = true;
    }

    /// Check if we're still in startup grace period
    fn in_startup_grace_period(&self) -> bool {
        Instant::now().duration_since(self.startup_time) < Duration::from_secs(120)
    }

    /// Update overall health status based on individual task health
    fn update_overall_health(&mut self) {
        // Initialize startup time on first call
        self.init_if_needed();

        // During startup grace period, don't start countdown
        if self.in_startup_grace_period() {
            return;
        }

        // Check which tasks are unhealthy
        let task_ids = [
            TaskId::Orchestrator,
            TaskId::Display,
            TaskId::AlarmTrigger,
            TaskId::TimeUpdater,
        ];
        let mut unhealthy_count = 0;

        for (index, task_id) in task_ids.iter().enumerate() {
            let task = &self.tasks[index];

            // Skip tasks that haven't reported yet (still initializing)
            if !task.has_reported {
                continue;
            }

            if !task.is_healthy(task_id.max_report_interval()) {
                unhealthy_count += 1;
                warn!("Task {:?} is unhealthy", task_id);
            }
        }

        let all_healthy = unhealthy_count == 0;

        // Start or reset countdown based on health status
        if all_healthy {
            // All monitored tasks are healthy - reset countdown
            if self.countdown_deadline.is_some() {
                info!("All tasks healthy - resetting countdown timer");
            }
            self.countdown_deadline = Some(Instant::now() + COUNTDOWN_TIMEOUT);
        } else if self.countdown_deadline.is_none() {
            // First detection of unhealthy tasks - start countdown
            warn!("{} task(s) unhealthy, starting countdown", unhealthy_count);
            self.countdown_deadline = Some(Instant::now() + COUNTDOWN_TIMEOUT);
        } else {
            // Countdown already running - just log status
            if let Some(remaining) = self.time_until_reset() {
                warn!(
                    "{} task(s) still unhealthy, {} seconds until reset",
                    unhealthy_count,
                    remaining.as_secs()
                );
            }
        }
    }

    /// Check if countdown has expired and we should trigger hardware watchdog
    fn should_trigger_reset(&self) -> bool {
        if self.in_startup_grace_period() {
            return false;
        }

        self.countdown_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    /// Get remaining time until reset
    fn time_until_reset(&self) -> Option<Duration> {
        self.countdown_deadline.map(|deadline| {
            let now = Instant::now();
            if now >= deadline {
                Duration::from_secs(0)
            } else {
                deadline - now
            }
        })
    }
}

/// Global system health tracker
static SYSTEM_HEALTH: Mutex<CriticalSectionRawMutex, SystemHealth> = Mutex::new(SystemHealth::new());

/// Report a successful task iteration
///
/// Only critical tasks should call this periodically to indicate they are functioning correctly.
///
/// Monitored tasks:
/// - Orchestrator: Must report regularly (event-driven, should be frequent)
/// - Display: Must report regularly (updates via signals)
/// - `AlarmTrigger`: Must report regularly (even while waiting for alarm)
/// - `TimeUpdater`: Must report after successful time sync (critical for alarm clock)
pub async fn report_task_success(task_id: TaskId) {
    let mut health = SYSTEM_HEALTH.lock().await;
    health.set_task_succeeded(task_id);
}

/// Report a failed task iteration
///
/// Tasks should call this when they encounter critical errors that might indicate
/// system instability. This immediately marks the task as unhealthy and will prevent
/// the countdown timer from resetting until the task reports success again.
pub async fn report_task_failure(task_id: TaskId) {
    warn!("Task {:?} reported failure", task_id);
    let mut health = SYSTEM_HEALTH.lock().await;
    let index = task_id as usize;
    // Clear the last report time to mark as unhealthy
    health.tasks[index].last_report = None;
    // Keep has_reported as true so we know it's initialized and should be checked
}

/// Watchdog task that monitors system health and triggers resets when needed
///
/// This task periodically checks the health of all monitored tasks. If all tasks
/// are healthy, it resets the countdown timer. If the countdown expires while
/// tasks are unhealthy, it triggers a hardware watchdog reset.
///
/// # Arguments
/// * `watchdog` - The watchdog peripheral from the RP2040
#[embassy_executor::task]
pub async fn watchdog_task(watchdog: Peri<'static, WATCHDOG>) {
    info!("Watchdog started - monitoring Orchestrator, Display, AlarmTrigger, TimeUpdater");
    info!(
        "Countdown: {}s, health checks every {}s, startup grace: 120s",
        COUNTDOWN_TIMEOUT.as_secs(),
        HEALTH_CHECK_INTERVAL.as_secs()
    );

    loop {
        // Check system health and update countdown
        let should_reset = {
            let mut health = SYSTEM_HEALTH.lock().await;
            health.update_overall_health();
            health.should_trigger_reset()
        };

        if should_reset {
            warn!("Countdown expired - system will reset due to unhealthy tasks");

            // Initialize hardware watchdog and don't feed it - this will cause reset
            let mut wd = Watchdog::new(watchdog);
            wd.pause_on_debug(false); // Don't pause during debug - we want the reset
            wd.start(HARDWARE_WATCHDOG_TIMEOUT);

            warn!(
                "Hardware watchdog started - system will reset in {}ms",
                HARDWARE_WATCHDOG_TIMEOUT.as_millis()
            );

            // Wait for hardware watchdog to reset the system
            loop {
                Timer::after_secs(1).await;
            }
        }

        // Wait before next health check
        Timer::after(HEALTH_CHECK_INTERVAL).await;
    }
}
