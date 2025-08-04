use bytes::BytesMut;

pub trait Stage {
    /// Returns the length of the data processed.
    fn process(&mut self, data: &mut BytesMut) -> usize;
}

pub struct Pipeline {
    stages: Vec<Box<dyn Stage>>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self { stages: Vec::new() }
    }

    pub fn add_stage<S: Stage + 'static>(mut self, stage: S) -> Self {
        self.stages.push(Box::new(stage));
        self
    }

    pub fn run_on_buf(&mut self, data: &mut BytesMut) {
        for stage in &mut self.stages {
            let len = stage.process(data);
            data.truncate(len);
        }
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;
    use bytes::BytesMut;

    /// Stage 1: ASCII uppercase in-place
    struct Uppercase;
    impl Stage for Uppercase {
        fn process(&mut self, data: &mut BytesMut) -> usize {
            for byte in &mut data[..] {
                *byte = byte.to_ascii_uppercase();
            }
            data.len()
        }
    }

    /// Stage 2: Reverse the bytes in-place
    struct Reverse;
    impl Stage for Reverse {
        fn process(&mut self, data: &mut BytesMut) -> usize {
            let len = data.len();
            for i in 0..len / 2 {
                data.swap(i, len - 1 - i);
            }
            len
        }
    }

    #[test]
    fn test_pipeline() {
        let mut pipeline = Pipeline::new().add_stage(Uppercase).add_stage(Reverse);

        let mut buf = BytesMut::new();
        buf.write_str("hello world").unwrap();
        pipeline.run_on_buf(&mut buf);

        assert_eq!(buf, BytesMut::from("DLROW OLLEH"));
    }
}
