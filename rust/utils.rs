use anyhow::Result;
use std::fmt;
use std::io::{Read, Write};

use super::config::Config;

pub trait Storage: Sized {
    fn save(&self, stream: &mut impl Write) -> Result<()>;
    fn load_with_version(stream: &mut impl Read, config: &Config, version: usize) -> Result<Self>;
}

pub fn bool_to_f64(b: bool) -> f64 {
    const T: f64 = f64::from_bits(!0);
    const F: f64 = f64::from_bits(0);
    if b {
        T
    } else {
        F
    }
}

/// aligns at a multiple of 32 (to cover different ABIs)
pub fn align_stack(n: u32) -> u32 {
    n + 16 - (n & 15)
}

/*****************************************/

#[derive(PartialEq)]
#[allow(unused)]
pub enum DataType {
    F32,
    F64,
}

#[derive(Copy, Clone, PartialEq, Hash, Eq)]
pub enum Reg {
    Ret,
    Temp,
    Gen(u8),
    Left,
    Right,
    Static(u32),
}

impl fmt::Debug for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reg::Ret => write!(f, "%$"),
            Reg::Left => write!(f, "%l"),
            Reg::Right => write!(f, "%r"),
            Reg::Temp => write!(f, "%t"),
            Reg::Gen(r) => write!(f, "%{}", r),
            Reg::Static(r) => write!(f, "σ{}", r),
        }
    }
}

pub fn reg(r: u8) -> Reg {
    Reg::Gen(r)
}
