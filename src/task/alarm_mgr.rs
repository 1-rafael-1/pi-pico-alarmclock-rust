use defmt::Format;

#[derive(Debug, Format)]
pub struct AlarmManager {
    pub state: AlarmState,
    pub time: u32,
    pub last_triggered: u32,
}

#[derive(PartialEq, Debug, Format)]
pub enum AlarmState {
    Idle,
    Armed,
    Triggered,
}

impl AlarmManager {
    pub fn new() -> Self {
        Self {
            state: AlarmState::Idle,
            time: 0,
            last_triggered: 0,
        }
    }

    pub fn arm(&mut self) {
        self.state = AlarmState::Armed;
    }

    pub fn disarm(&mut self) {
        self.state = AlarmState::Idle;
    }

    pub fn trigger(&mut self) {
        self.state = AlarmState::Triggered;
    }
}
