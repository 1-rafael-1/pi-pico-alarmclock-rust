//! # Sound task
//!  This module contains the task that plays sound using the `DFPlayer` Mini module.
//!
//! The task is responsible for initializing the `DFPlayer` Mini module, powering it on, playing a sound, and powering it off.
use defmt::{Debug2Format, info};
use dfplayer_async::{DfPlayer, Equalizer, PlayBackSource, TimeSource};
use embassy_rp::{gpio::Output, uart::BufferedUart};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Delay, Duration, Instant, Timer};

/// Signal for starting the sound
static SOUND_START_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal for stopping the sound
static SOUND_STOP_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signals the sound task to start playing
pub fn signal_sound_start() {
    SOUND_START_SIGNAL.signal(());
}

/// Signals the sound task to stop playing
pub fn signal_sound_stop() {
    SOUND_STOP_SIGNAL.signal(());
}

/// Waits for the next sound start signal
async fn wait_for_sound_start() {
    SOUND_START_SIGNAL.wait().await;
}

/// Waits for the next sound stop signal
async fn wait_for_sound_stop() {
    SOUND_STOP_SIGNAL.wait().await;
}

// Time source implementation for DFPlayer
/// Time source implementation for the `DFPlayer` using Embassy's `Instant`.
struct MyTimeSource;

impl TimeSource for MyTimeSource {
    type Instant = Instant;

    fn now(&self) -> Self::Instant {
        Instant::now()
    }

    fn is_elapsed(&self, since: Self::Instant, timeout_ms: u64) -> bool {
        Instant::now().duration_since(since) >= Duration::from_millis(timeout_ms)
    }
}

#[embassy_executor::task]
pub async fn sound_handler(mut uart: BufferedUart, mut pwr: Output<'static>) {
    info!("Sound task started");

    let feedback_enable = false;
    let timeout = Duration::from_secs(1);
    let reset_duration_override = Some(Duration::from_millis(1000));

    loop {
        // wait for the signal to start playing sound
        wait_for_sound_start().await;

        // power on the dfplayer
        info!("Powering on the dfplayer");
        pwr.set_high();
        Timer::after(Duration::from_millis(500)).await;
        info!("Powered on the dfplayer");

        let time_source = MyTimeSource;
        let delay = Delay;
        let mut dfp_result = DfPlayer::new(
            &mut uart,
            feedback_enable,
            timeout.as_millis(),
            time_source,
            delay,
            reset_duration_override.map(|d| d.as_millis()),
        )
        .await;

        match dfp_result {
            Ok(_) => info!("DfPlayer initialized successfully"),
            Err(ref e) => info!("DfPlayer initialization failed with error {:?}", Debug2Format(&e)),
        }

        info!("Playing sound");
        if let Ok(ref mut dfp) = dfp_result {
            let _ = dfp.set_volume(13).await;
            Timer::after(Duration::from_millis(100)).await;
            let _ = dfp.set_equalizer(Equalizer::Classic).await;
            Timer::after(Duration::from_millis(100)).await;
            let _ = dfp.set_playback_source(PlayBackSource::SDCard).await;
            Timer::after(Duration::from_millis(100)).await;
            let _ = dfp.play(1).await;
            Timer::after(Duration::from_millis(200)).await;
        } else {
            info!("DfPlayer not initialized, skipping sound playback.");
        }

        // wait for the signal to stop playing sound
        wait_for_sound_stop().await;

        // power off the dfplayer
        info!("Powering off the dfplayer");
        pwr.set_low();
    }
}
