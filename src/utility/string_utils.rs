use heapless::String;

pub struct StringUtils;

impl StringUtils {
    /// This function converts a &str to a heapless::String<128>. Apparently simple strings are not really working in embedded systems
    pub fn convert_str_to_heapless_safe(s: &str) -> Result<String<128>, &'static str> {
        let mut heapless_string: String<128> = String::new();
        for c in s.chars() {
            if heapless_string.push(c).is_err() {
                return Err("String exceeds capacity");
            }
        }
        Ok(heapless_string)
    }

    /// This function unwraps a heapless::String<128> or returns an empty heapless::String<128> if None.
    pub fn unwrap_or_default_heapless_string(s: Option<String<128>>) -> String<128> {
        match s {
            Some(value) => value,  // Directly return the heapless::String<128>
            None => String::new(), // Return an empty heapless::String if None
        }
    }
}
