use crate::convert::to_archive::yinmodel_to_archive;
use crate::model::YinModel;

/// Save a `YinModel` to a `.yin` file.
pub fn save_yin(model: &YinModel, path: &str) -> std::io::Result<()> {
    let archive = yinmodel_to_archive(model);
    archive.write_to(path)
}
