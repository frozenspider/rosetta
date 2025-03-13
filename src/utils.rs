use unicode_segmentation::UnicodeSegmentation;

pub fn substr_up_to_len(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        s.graphemes(true).take(max_len).collect::<String>()
    } else {
        s.to_owned()
    }
}
