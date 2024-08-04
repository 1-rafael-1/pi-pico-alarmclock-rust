//! # StringUtils
//! This module contains utility functions around string handling that are used in the project.

use core::fmt::Write;
use embassy_rp::rtc::{DateTime, DayOfWeek};
use heapless::String;
use heapless::Vec;

pub struct StringUtils;

impl StringUtils {
    /// This function converts a &str to a DateTime struct
    /// The input string should be in the format "YYYY-MM-DDTHH:MM:SS.ssssss+HH:MM"
    /// one example being "2024-06-26T22:01:27.106426+02:00"
    pub fn convert_str_to_datetime(s: &str, d: u8) -> DateTime {
        const CAPACITY: usize = 10;

        let mut dt = DateTime {
            year: 0,
            month: 0,
            day: 0,
            day_of_week: match d {
                1 => DayOfWeek::Monday,
                2 => DayOfWeek::Tuesday,
                3 => DayOfWeek::Wednesday,
                4 => DayOfWeek::Thursday,
                5 => DayOfWeek::Friday,
                6 => DayOfWeek::Saturday,
                0 => DayOfWeek::Sunday, // as specified by worldtimeapi.org
                _ => DayOfWeek::Monday,
            },
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

    /// This function converts a DateTime struct to a string
    /// The output string will be in the format "DayOfWeek DD.MM.YYYY", with padding to center the string in a 22 character field
    /// one example being `" Saturday 26.06.2024  "`
    pub fn convert_datetime_to_str(dt: DateTime) -> String<22> {
        let mut s: String<20> = String::new();
        let _ = write!(
            s,
            "{:?} {:02}.{:02}.{}",
            dt.day_of_week, dt.day, dt.month, dt.year
        );

        let content_length = s.chars().count();
        let total_length: u8 = 22;
        let padding = total_length - content_length as u8;
        let padding_left = padding / 2;
        let padding_right = padding - padding_left;

        let mut padded_string: String<22> = String::new();
        for _ in 0..padding_left {
            padded_string.push(' ').unwrap();
        }
        padded_string.push_str(&s).unwrap();
        for _ in 0..padding_right {
            padded_string.push(' ').unwrap();
        }
        padded_string
    }
}
