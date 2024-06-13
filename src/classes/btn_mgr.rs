use defmt::info;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Input, Level};
use embassy_time::{with_deadline, Duration, Instant, Timer};
use {defmt_rtt as _, panic_probe as _};

pub struct ButtonManager<'a> {
    input: Input<'a>,
    debounce_duration: Duration,
    id: &'a str,
}

impl<'a> ButtonManager<'a> {
    pub fn new(input: Input<'a>, debounce_duration: Duration, id: &'a str) -> Self {
        Self {
            input,
            debounce_duration,
            id,
        }
    }

    pub async fn handle_button_press(&mut self) {
        loop {
            // button pressed
            let _debounce = self.debounce().await;
            let start = Instant::now();
            info!("{} Press", self.id);

            match with_deadline(start + Duration::from_secs(1), self.debounce()).await {
                // Button Released < 1s
                Ok(_) => {
                    info!("{} pressed for: {}ms", self.id, start.elapsed().as_millis());
                    continue;
                }
                // button held for > 1s
                Err(_) => {
                    info!("{} Held", self.id);
                }
            }

            match with_deadline(start + Duration::from_secs(5), self.debounce()).await {
                // Button released <5s
                Ok(_) => {
                    info!("{} pressed for: {}ms", self.id, start.elapsed().as_millis());
                    continue;
                }
                // button held for > >5s
                Err(_) => {
                    info!("{} Long Held", self.id);
                }
            }

            // wait for button release before handling another press
            self.debounce().await;
            info!("{} pressed for: {}ms", self.id, start.elapsed().as_millis());
        }
    }

    pub async fn debounce(&mut self) -> Level {
        loop {
            let l1 = self.input.get_level();

            self.input.wait_for_any_edge().await;

            Timer::after(self.debounce_duration).await;

            let l2 = self.input.get_level();
            if l1 != l2 {
                break l2;
            }
        }
    }
}

#[embassy_executor::task]
pub async fn green_button(_spawner: Spawner, input: Input<'static>) {
    let mut btn = ButtonManager::new(input, Duration::from_millis(100), "green_button");
    info!("{} task started", btn.id);
    btn.handle_button_press().await;
}

#[embassy_executor::task]
pub async fn yellow_button(_spawner: Spawner, input: Input<'static>) {
    let mut btn = ButtonManager::new(input, Duration::from_millis(100), "yellow_button");
    info!("{} task started", btn.id);
    btn.handle_button_press().await;
}

#[embassy_executor::task]
pub async fn blue_button(_spawner: Spawner, input: Input<'static>) {
    let mut btn = ButtonManager::new(input, Duration::from_millis(100), "blue_button");
    info!("{} task started", btn.id);
    btn.handle_button_press().await;
}
