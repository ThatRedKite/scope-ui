use std::fmt::Display;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum OscilloscopeError {
    S1Failure,
    WriteError,
    RiError,
    RoError
}

impl Display for OscilloscopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let a = match *self {
            Self::RiError  => "capture error",
            Self::S1Failure     => "S1 failure",
            Self::WriteError    => "write error",
            Self::RoError=> "measurement condition error"
        };
        write!(f, "{a}")
    }
}

