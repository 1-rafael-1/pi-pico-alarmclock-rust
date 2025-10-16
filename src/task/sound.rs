//! # Sound task
//!  This module contains the task that plays sound using the DFPlayer Mini module.
//!
//! The task is responsible for initializing the DFPlayer Mini module, powering it on, playing a sound, and powering it off.
use crate::task::task_messages::{SOUND_START_SIGNAL, SOUND_STOP_SIGNAL};
use defmt::{Debug2Format, info};
use dfplayer_async::{DfPlayer, Equalizer, PlayBackSource, TimeSource};
use embassy_rp::gpio::Output;
use embassy_rp::uart::BufferedUart;
use embassy_time::{Delay, Duration, Instant, Timer};

// Time source implementation for DFPlayer
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

    let feedback_enable = false; // fails to acknoweledge when enabled
    let timeout = Duration::from_secs(1);
    let reset_duration_override = Some(Duration::from_millis(1000));

    loop {
        // wait for the signal to start playing sound
        SOUND_START_SIGNAL.wait().await;

        // power on the dfplayer
        info!("Powering on the dfplayer");
        pwr.set_high();
        Timer::after(Duration::from_secs(1)).await;
        info!("Powered on the dfplayer");

        let time_source = MyTimeSource;
        let delay = Delay;
        let mut dfp_result = DfPlayer::new(
            &mut uart,
            feedback_enable,
            timeout.as_millis() as u64,
            time_source,
            delay,
            reset_duration_override.map(|d| d.as_millis() as u64),
        )
        .await;

        match dfp_result {
            Ok(_) => info!("DfPlayer initialized successfully"),
            Err(ref e) => info!(
                "DfPlayer initialization failed with error {:?}",
                Debug2Format(&e)
            ),
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
        SOUND_STOP_SIGNAL.wait().await;

        // power off the dfplayer
        info!("Powering off the dfplayer");
        pwr.set_low();
    }
}
