#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum Channel {
    DISPLAY1 = 1,
    DISPLAY2 = 2,
    SAVE1 = 3,
    SAVE2 = 4
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }    
}
