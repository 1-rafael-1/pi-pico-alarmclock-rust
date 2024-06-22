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

    /// This function concatenates two heapless::String<128> and <128> into a heapless::String<256>
    pub fn concatenate_heapless_strings(
        first_string: &heapless::String<128>,
        second_string: &heapless::String<128>,
    ) -> heapless::String<256> {
        let mut combined = String::<256>::new();
        combined.push_str(first_string.as_str()).unwrap_or_default();
        combined
            .push_str(second_string.as_str())
            .unwrap_or_default();
        combined
    }
}
