use num_complex::Complex;

#[derive(Clone, Debug)]
pub enum ElemType {
    RealF64(f64),
    ComplexF64(Complex<f64>),
}

/// The number types of `Application::interpret`: `f64` and `Complex<f64>`.
pub trait Element: Default {
    fn get_type(x: Self) -> ElemType;
}

impl Element for f64 {
    fn get_type(x: Self) -> ElemType {
        ElemType::RealF64(x)
    }
}

impl Element for Complex<f64> {
    fn get_type(x: Self) -> ElemType {
        ElemType::ComplexF64(x)
    }
}
