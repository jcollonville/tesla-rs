// SPDX-License-Identifier: MIT OR Apache-2.0

/// How far behind the newest response counter a client still remembers.
const WINDOW_BITS: u32 = 32;

/// Per-request response counters.
///
/// The first counter is accepted. Later counters must be unique and no more than 32 behind the
/// highest accepted counter. A shift of 64 or more clears the history instead of masking the
/// shift, which is what a `u64` shift would do in release mode.
#[derive(Clone, Debug, Default)]
pub(crate) struct ReplayWindow {
    history: u64,
    counter: u32,
    used: bool,
}

impl ReplayWindow {
    pub(crate) fn update(&mut self, new_counter: u32) -> bool {
        if !self.used {
            self.used = true;
            self.counter = new_counter;
            return true;
        }
        let (counter, history, ok) = update_window(self.counter, self.history, new_counter);
        if ok {
            self.counter = counter;
            self.history = history;
        }
        ok
    }
}

fn update_window(counter: u32, window: u64, new_counter: u32) -> (u32, u64, bool) {
    if counter == new_counter {
        return (counter, window, false);
    }
    if new_counter < counter {
        let age = counter - new_counter;
        if age > WINDOW_BITS {
            return (counter, window, false);
        }
        let bit = age - 1;
        if (window >> bit) & 1 == 1 {
            return (counter, window, false);
        }
        return (counter, window | (1_u64 << bit), true);
    }
    let shift = u64::from(new_counter - counter);
    let mut updated = if shift >= 64 { 0 } else { window << shift };
    if shift > 0 && shift - 1 < 64 {
        updated |= 1_u64 << (shift - 1);
    }
    (new_counter, updated, true)
}

#[cfg(test)]
mod tests {
    use super::update_window;

    #[test]
    fn sliding_window_matches_reference_vectors() {
        let cases = [
            (
                100,
                (1_u64 << 0) | (1 << 5),
                101,
                101,
                1 | (1 << 1) | (1 << 6),
                true,
            ),
            (
                100,
                (1_u64 << 0) | (1 << 5),
                103,
                103,
                (1 << 2) | (1 << 3) | (1 << 8),
                true,
            ),
            (100, (1_u64 << 0) | (1 << 5), 500, 500, 0, true),
            (
                100,
                (1_u64 << 0) | (1 << 5),
                98,
                100,
                (1 << 0) | (1 << 1) | (1 << 5),
                true,
            ),
            (
                100,
                (1_u64 << 0) | (1 << 5),
                99,
                100,
                (1 << 0) | (1 << 5),
                false,
            ),
            (
                100,
                (1_u64 << 0) | (1 << 5),
                3,
                100,
                (1 << 0) | (1 << 5),
                false,
            ),
            (
                100,
                (1_u64 << 0) | (1 << 5),
                100,
                100,
                (1 << 0) | (1 << 5),
                false,
            ),
        ];
        for (counter, window, new_counter, expect_counter, expect_window, expect_ok) in cases {
            let (got_counter, got_window, ok) = update_window(counter, window, new_counter);
            assert_eq!(got_counter, expect_counter);
            assert_eq!(got_window, expect_window);
            assert_eq!(ok, expect_ok);
        }
    }
}
