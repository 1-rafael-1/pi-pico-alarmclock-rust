// make sure to have a time_api_config.json file in the config folder formatted as follows:
include!(concat!(env!("OUT_DIR"), "/time_api_config.rs"));

use crate::utility::string_utils::StringUtils;
//use embassy_net::Stack;
//use embassy_rp::rtc::{DateTime, DayOfWeek, Instance, Rtc};
use embassy_rp::rtc::Instance;
use embassy_rp::rtc::Rtc;
//use embassy_time::{Duration, Timer};
use heapless::String;

pub struct TimeManager<'a, T: Instance> {
    real_time_clock: Rtc<'a, T>,
    time_server_url: Option<String<128>>,
}

impl<'a, T: embassy_rp::rtc::Instance> TimeManager<'a, T> {
    pub fn new(rtc: Rtc<'a, T>) -> Self {
        let mut manager = TimeManager {
            real_time_clock: rtc,
            time_server_url: None,
        };
        manager.set_time_server_url();
        manager
    }

    fn set_time_server_url(&mut self) {
        self.time_server_url =
            Some(StringUtils::convert_str_to_heapless_safe(TIME_SERVER_URL).unwrap());
    }

    // pub async fn update_rtc(&self) {
    //     let url = StringUtils::unwrap_or_default_heapless_string(self.time_server_url.clone());
    //     let zone = StringUtils::unwrap_or_default_heapless_string(self.time_zone.clone());
    //     let combined_url = StringUtils::concatenate_heapless_strings(&url, &zone);
    // }
}
