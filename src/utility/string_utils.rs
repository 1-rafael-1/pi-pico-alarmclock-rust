//! # `StringUtils`
//! This module contains utility functions around string handling that are used in the project.

use core::fmt::Write;

use embassy_rp::rtc::DateTime;
use heapless::String;

/// A utility struct for string operations
pub struct StringUtils;

impl StringUtils {
    /// This function converts a `DateTime` struct to a string
    /// The output string will be in the format "`DayOfWeek` DD.MM.YYYY", with padding to center the string in a 22 character field
    /// one example being `" Saturday 26.06.2024  "`
    ///
    /// If padding fails due to capacity constraints, the padding is skipped.
    pub fn convert_datetime_to_str(dt: &DateTime) -> String<22> {
        let mut s: String<20> = String::new();
        let _ = write!(s, "{:?} {:02}.{:02}.{}", dt.day_of_week, dt.day, dt.month, dt.year);

        let content_length = s.chars().count();
        let total_length: u8 = 22;
        #[allow(clippy::cast_possible_truncation)]
        let padding = total_length - content_length as u8;
        let padding_left = padding / 2;
        let padding_right = padding - padding_left;

        let mut padded_string: String<22> = String::new();

        // Add left padding (skip if it fails)
        for _ in 0..padding_left {
            let _ = padded_string.push(' ');
        }

        // Add the content (skip if it fails, though unlikely)
        let _ = padded_string.push_str(&s);

        // Add right padding (skip if it fails)
        for _ in 0..padding_right {
            let _ = padded_string.push(' ');
        }

        padded_string
    }
}
