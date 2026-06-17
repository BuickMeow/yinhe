use crate::convert::from_archive::archive_to_yinmodel;
use crate::model::YinModel;

/// Load a `.yin` file and return a `YinModel`.
pub fn load_yin(path: &str) -> std::io::Result<YinModel> {
    let archive = yinhe_project::ProjectArchive::read_from(path)?;
    Ok(archive_to_yinmodel(&archive))
}
