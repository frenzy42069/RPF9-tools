pub fn align16(x: usize) -> usize {
    (x + 15) & !15
}

/// Inverse-image of `rposition` on a range of the file: the last index `i`
/// within `range` where `data[i] != 0`, or `range.start - 1` if none.
pub fn last_nonzero(data: &[u8], range: std::ops::Range<usize>) -> usize {
    let Some(i) = data[range.clone()].iter().rposition(|&b| b != 0) else {
        return range.start.saturating_sub(1);
    };
    range.start + i
}
