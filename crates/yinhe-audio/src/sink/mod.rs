pub mod cpal;
pub mod wav;

pub trait AudioSink {
    fn write(&mut self, samples: &[f32]);
    fn flush(&mut self);
}
