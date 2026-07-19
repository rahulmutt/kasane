#[derive(Clone, Debug)]
pub struct Options {
    pub max_tokens: usize,
    pub min_tokens: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            max_tokens: 2000,
            min_tokens: 200,
        }
    }
}
