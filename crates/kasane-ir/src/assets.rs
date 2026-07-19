#[derive(Clone, Debug, Default)]
pub struct AssetBag {
    pub items: Vec<AssetItem>,
}

#[derive(Clone, Debug)]
pub struct AssetItem {
    pub key: String,
    pub filename: String,
    pub bytes: Vec<u8>,
}
