use crate::iir::IirFilter;
use num::complex::Complex64;
use num::{One, Zero};
use std::array;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterType {
    LowPass,
    HighPass,
}

#[must_use]
pub fn butterworth<const N: usize>(fc: f64, fs: f64, filter: FilterType) -> IirFilter<N> {
    let wc = fc / (fs / 2.0);
    if !(0.0..=1.0).contains(&wc) {
        log::error!(
            "Attempted to design order {N} {filter:?} Butterworth filter with invalid frequencies, replacing with identity filter: fc={fc}, fs={fs}"
        );
        return IirFilter::identity();
    }

    let (b, a) = butterworth_coefficients::<N>(fc, fs, filter);
    IirFilter::new(&b, &a)
}

#[allow(clippy::many_single_char_names)]
fn butterworth_coefficients<const N: usize>(
    fc: f64,
    fs: f64,
    filter: FilterType,
) -> (Vec<f64>, Vec<f64>) {
    use std::f64::consts::PI;

    let n = N as f64;

    // Compute Butterworth poles for low-pass prototype
    let poles: [_; N] = array::from_fn(|i| {
        let k = (i + 1) as f64;
        // TJE native: exp(i * angle) avoids ComplexFloat's platform-dependent
        // hypot/atan2/log/exp route. Preserve the original angle arithmetic.
        let angle = PI * (2.0 * k + n - 1.0) / (2.0 * n);
        Complex64::new(libm::cos(angle), libm::sin(angle))
    });

    // Warp analog frequency and convert low-pass prototype poles to poles for desired filter type
    let wc = fc / (fs / 2.0);
    let warp = 2.0 * libm::tan(wc * PI / 2.0);
    let poles = match filter {
        FilterType::LowPass => poles.map(|p| warp * p),
        FilterType::HighPass => poles.map(|p| warp / p),
    };

    // Perform bilinear transform
    let poles = poles.map(|p| (1.0 + p / 2.0) / (1.0 - p / 2.0));

    // Compute base feedforward coefficients
    let zeroes = match filter {
        FilterType::LowPass => [Complex64::new(-1.0, 0.0); N],
        FilterType::HighPass => [Complex64::new(1.0, 0.0); N],
    };
    let b = polynomial_coefficients(zeroes);

    // Compute feedback coefficients
    let a = polynomial_coefficients(poles);

    // Normalize feedforward coefficients
    let k = match filter {
        FilterType::LowPass => a.iter().copied().sum::<f64>() / b.iter().copied().sum::<f64>(),
        FilterType::HighPass => {
            let high_pass_sum = |arr: &[f64]| {
                arr.iter()
                    .copied()
                    .enumerate()
                    .map(|(i, n)| if i & 1 == 0 { n } else { -n })
                    .sum::<f64>()
            };

            high_pass_sum(&a) / high_pass_sum(&b)
        }
    };
    let b: Vec<_> = b.into_iter().map(|b| b * k).collect();

    log::debug!("Filter for fc={fc}, fs={fs}, type {filter:?}:");
    log::debug!("  b={b:?}");
    log::debug!("  a={a:?}");

    (b, a)
}

fn polynomial_coefficients<const N: usize>(roots: [Complex64; N]) -> Vec<f64> {
    (0..=N)
        .map(|i| {
            let sign = if i & 1 == 0 { 1.0 } else { -1.0 };
            (sign * sum_combinations(Complex64::one(), &roots, i)).re
        })
        .collect()
}

fn sum_combinations(product: Complex64, roots: &[Complex64], len: usize) -> Complex64 {
    if len == 0 {
        return product;
    }

    if roots.len() < len {
        return Complex64::zero();
    }

    (0..roots.len()).map(|i| sum_combinations(product * roots[i], &roots[i + 1..], len - 1)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::iter;

    #[test]
    fn device_rate_first_and_second_order_coefficients_follow_software_poles() {
        use std::f64::consts::PI;
        for fs in [53_693_175.0 / 7.0 / 6.0 / 24.0, 53_693_175.0 / 15.0 / 16.0] {
            for fc in [5.0, 3390.0, 15750.0] {
                for filter in [FilterType::LowPass, FilterType::HighPass] {
                    let warp = 2.0 * libm::tan((fc / (fs / 2.0)) * PI / 2.0);
                    let pole = |angle| {
                        let p = Complex64::new(libm::cos(angle), libm::sin(angle));
                        let p = match filter {
                            FilterType::LowPass => warp * p,
                            FilterType::HighPass => warp / p,
                        };
                        (1.0 + p / 2.0) / (1.0 - p / 2.0)
                    };
                    let first = pole(PI);
                    let second = [pole(PI * 3.0 / 4.0), pole(PI * 5.0 / 4.0)];
                    let sign = match filter {
                        FilterType::LowPass => 1.0,
                        FilterType::HighPass => -1.0,
                    };
                    let cases = [
                        (
                            butterworth_coefficients::<1>(fc, fs, filter),
                            vec![1.0, -first.re],
                            vec![1.0, sign],
                        ),
                        (
                            butterworth_coefficients::<2>(fc, fs, filter),
                            vec![1.0, -(second[0] + second[1]).re, (second[0] * second[1]).re],
                            vec![1.0, 2.0 * sign, 1.0],
                        ),
                    ];
                    for ((b, a), expected_a, raw_b) in cases {
                        let at_nyquist = |values: &[f64]| match values {
                            [a0, a1] => a0 - a1,
                            [a0, a1, a2] => a0 - a1 + a2,
                            _ => panic!("fixture requires a first- or second-order polynomial"),
                        };
                        let gain = match filter {
                            FilterType::LowPass => {
                                expected_a.iter().copied().sum::<f64>()
                                    / raw_b.iter().copied().sum::<f64>()
                            }
                            FilterType::HighPass => at_nyquist(&expected_a) / at_nyquist(&raw_b),
                        };
                        assert_eq!(
                            a.into_iter().map(f64::to_bits).collect::<Vec<_>>(),
                            expected_a.into_iter().map(f64::to_bits).collect::<Vec<_>>()
                        );
                        assert_eq!(
                            b.into_iter().map(f64::to_bits).collect::<Vec<_>>(),
                            raw_b.into_iter().map(|b| (b * gain).to_bits()).collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
    }

    // Historical Linux diagnostic only: this is deliberately not a portability
    // requirement, and none of the platform-libm calls below ship in the library.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn historical_linux_coefficients<const N: usize>(
        fc: f64,
        fs: f64,
        filter: FilterType,
    ) -> (Vec<f64>, Vec<f64>) {
        use num::complex::ComplexFloat;
        use std::f64::consts::{E, PI};
        let n = N as f64;
        let j = Complex64::i();
        let poles: [_; N] = array::from_fn(|i| {
            let k = (i + 1) as f64;
            E.powc(j * PI * (2.0 * k + n - 1.0) / (2.0 * n))
        });
        let wc = fc / (fs / 2.0);
        let warp = 2.0 * (wc * PI / 2.0).tan();
        let poles = match filter {
            FilterType::LowPass => poles.map(|p| warp * p),
            FilterType::HighPass => poles.map(|p| warp / p),
        };
        let poles = poles.map(|p| (1.0 + p / 2.0) / (1.0 - p / 2.0));
        let zeroes = match filter {
            FilterType::LowPass => [Complex64::new(-1.0, 0.0); N],
            FilterType::HighPass => [Complex64::new(1.0, 0.0); N],
        };
        let polynomial = |roots: [Complex64; N]| {
            (0..=N)
                .map(|i| {
                    ((-1.0_f64).powi(i as i32) * sum_combinations(Complex64::one(), &roots, i)).re
                })
                .collect::<Vec<_>>()
        };
        let b = polynomial(zeroes);
        let a = polynomial(poles);
        let gain = match filter {
            FilterType::LowPass => a.iter().copied().sum::<f64>() / b.iter().copied().sum::<f64>(),
            FilterType::HighPass => {
                let alternating = |values: &[f64]| {
                    values
                        .iter()
                        .copied()
                        .enumerate()
                        .map(|(i, value)| (-1.0_f64).powi(i as i32) * value)
                        .sum::<f64>()
                };
                alternating(&a) / alternating(&b)
            }
        };
        (b.into_iter().map(|value| value * gain).collect(), a)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    #[ignore = "historical Linux bit diagnostic, not a cross-target acceptance rule"]
    fn historical_linux_math_default_filter_bits() {
        for (mode, mclk) in [("ntsc", 53_693_175.0), ("pal", 53_203_424.0)] {
            for (chip, fs, is_ym) in
                [("ym2612", mclk / 7.0 / 6.0 / 24.0, true), ("psg", mclk / 15.0 / 16.0, false)]
            {
                // Config defaults: 5-Hz DC rejection and 3390-Hz first LPF.
                let mut cases = vec![
                    (
                        butterworth_coefficients::<1>(5.0, fs, FilterType::HighPass),
                        historical_linux_coefficients::<1>(5.0, fs, FilterType::HighPass),
                    ),
                    (
                        butterworth_coefficients::<1>(3390.0, fs, FilterType::LowPass),
                        historical_linux_coefficients::<1>(3390.0, fs, FilterType::LowPass),
                    ),
                ];
                if is_ym {
                    // Also compare the constructed (default-disabled) second LPF.
                    cases.push((
                        butterworth_coefficients::<2>(6725.0, fs, FilterType::LowPass),
                        historical_linux_coefficients::<2>(6725.0, fs, FilterType::LowPass),
                    ));
                }
                let mut coefficients = 0;
                let mut changed = 0;
                for ((b, a), (old_b, old_a)) in cases {
                    assert_eq!(b.len(), old_b.len());
                    assert_eq!(a.len(), old_a.len());
                    for (new, old) in b.iter().chain(&a).zip(old_b.iter().chain(&old_a)) {
                        coefficients += 1;
                        changed += usize::from(new.to_bits() != old.to_bits());
                    }
                }
                eprintln!(
                    "HISTORICAL_LINUX_MATH {mode}/{chip}: {coefficients} coefficient bits compared, {changed} changed"
                );
            }
        }
    }

    fn float_slice_equal(a: &[f64], b: &[f64]) -> bool {
        if a.len() != b.len() {
            return false;
        }

        iter::zip(a, b).all(|(&a_elem, &b_elem)| (a_elem - b_elem).abs() < 1e-9)
    }

    fn assert_float_slice_eq(a: &[f64], b: &[f64]) {
        assert!(float_slice_equal(a, b), "float slices not equal: {a:?} {b:?}");
    }

    #[test]
    fn butterworth_low_pass() {
        // Expected filters generated using Python w/ scipy:
        //   b, a = butter(n, 3390 / (53693175 / 7 / 6 / 24 / 2), btype="lowpass")

        const B1: &[f64] = &[0.1684983368367697, 0.1684983368367697];
        const A1: &[f64] = &[1.0, -0.6630033263264605];

        const B2: &[f64] = &[0.030930211590861196, 0.06186042318172239, 0.030930211590861196];
        const A2: &[f64] = &[1.0, -1.4445658935949237, 0.5682867399583684];

        const B3: &[f64] = &[
            0.005563425334839113,
            0.016690276004517342,
            0.016690276004517342,
            0.005563425334839113,
        ];
        const A3: &[f64] = &[1.0, -2.2050627555803564, 1.696520707984555, -0.44695054972548554];

        let fc = 3390.0;
        let fs = 53693175.0 / 7.0 / 6.0 / 24.0;

        let (b1, a1) = butterworth_coefficients::<1>(fc, fs, FilterType::LowPass);
        assert_float_slice_eq(&b1, B1);
        assert_float_slice_eq(&a1, A1);

        let (b2, a2) = butterworth_coefficients::<2>(fc, fs, FilterType::LowPass);
        assert_float_slice_eq(&b2, B2);
        assert_float_slice_eq(&a2, A2);

        let (b3, a3) = butterworth_coefficients::<3>(fc, fs, FilterType::LowPass);
        assert_float_slice_eq(&b3, B3);
        assert_float_slice_eq(&a3, A3);
    }

    #[test]
    fn butterworth_high_pass() {
        // Expected filters generated using Python w/ scipy:
        //   b, a = butter(n, 3390 / (53693175 / 7 / 6 / 24 / 2), btype="highpass")

        const B1: &[f64] = &[0.8315016631632303, -0.8315016631632303];
        const A1: &[f64] = &[1.0, -0.6630033263264605];

        const B2: &[f64] = &[0.753213158388323, -1.506426316776646, 0.753213158388323];
        const A2: &[f64] = &[1.0, -1.4445658935949237, 0.5682867399583684];

        const B3: &[f64] =
            &[0.6685667516612996, -2.005700254983899, 2.005700254983899, -0.6685667516612996];
        const A3: &[f64] = &[1.0, -2.2050627555803564, 1.696520707984555, -0.4469505497254855];

        let fc = 3390.0;
        let fs = 53693175.0 / 7.0 / 6.0 / 24.0;

        let (b1, a1) = butterworth_coefficients::<1>(fc, fs, FilterType::HighPass);
        assert_float_slice_eq(&b1, B1);
        assert_float_slice_eq(&a1, A1);

        let (b2, a2) = butterworth_coefficients::<2>(fc, fs, FilterType::HighPass);
        assert_float_slice_eq(&b2, B2);
        assert_float_slice_eq(&a2, A2);

        let (b3, a3) = butterworth_coefficients::<3>(fc, fs, FilterType::HighPass);
        assert_float_slice_eq(&b3, B3);
        assert_float_slice_eq(&a3, A3);
    }
}
