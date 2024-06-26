use embassy_rp::rtc::{DateTime, DayOfWeek};
use heapless::Vec;

pub struct StringUtils;

impl StringUtils {
    /// This function converts a &str to a DateTime struct
    /// The input string should be in the format "YYYY-MM-DDTHH:MM:SS.ssssss+HH:MM"
    /// one example being "2024-06-26T22:01:27.106426+02:00"
    pub fn convert_str_to_datetime(s: &str) -> DateTime {
        const CAPACITY: usize = 10;

        let mut dt = DateTime {
            year: 0,
            month: 0,
            day: 0,
            day_of_week: DayOfWeek::Monday, // default to Monday, we don't care about the day of the week
            hour: 0,
            minute: 0,
            second: 0,
        };

        // Split the input string into date and time components
        let parts: Vec<&str, CAPACITY> = s.split('T').collect();
        if parts.len() == 2 {
            // Process the date part
            let date_parts: Vec<&str, CAPACITY> = parts[0].split('-').collect();
            if date_parts.len() == 3 {
                dt.year = date_parts[0].parse::<u16>().unwrap_or_default();
                dt.month = date_parts[1].parse::<u8>().unwrap_or_default();
                dt.day = date_parts[2].parse::<u8>().unwrap_or_default();
            }

            // Process the time part, ignoring fractional seconds and timezone
            let time_parts: Vec<&str, CAPACITY> = parts[1].split(':').collect();
            if time_parts.len() >= 3 {
                dt.hour = time_parts[0].parse::<u8>().unwrap_or_default();
                dt.minute = time_parts[1].parse::<u8>().unwrap_or_default();
                // Extract seconds, ignoring fractional part
                let second_parts: Vec<&str, CAPACITY> = time_parts[2].split('.').collect();
                dt.second = second_parts[0].parse::<u8>().unwrap_or_default();
            }
        }
        dt
    }
}
