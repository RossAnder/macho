//! Decimal formatting without `core::fmt`, whose machinery costs far more than the
//! digits for the handful of numbers a fetch prints.

/// A number's decimal digits, formatted on the stack.
pub struct Decimal {
    buf: [u8; 20],
    start: usize,
}

impl Decimal {
    pub fn new(mut n: u64) -> Self {
        let mut buf = [0; 20];
        let mut start = buf.len();
        loop {
            start -= 1;
            buf[start] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                return Self { buf, start };
            }
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[self.start..]
    }

    pub fn as_str(&self) -> &str {
        // SAFETY: the buffer from `start` on holds ASCII digits only.
        unsafe { core::str::from_utf8_unchecked(self.as_bytes()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_display() {
        for n in [0, 1, 9, 10, 99, 100, 1065, 4_294_967_295, u64::MAX] {
            assert_eq!(Decimal::new(n).as_str(), n.to_string());
        }
    }
}
