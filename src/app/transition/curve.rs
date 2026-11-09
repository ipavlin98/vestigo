/// The minimum-jerk profile (Flash and Hogan, 1985): the path of least change in acceleration
/// between two resting points, `10t³ − 15t⁴ + 6t⁵`. Its speed rises and falls as a symmetric
/// bell, starting and ending without a kick and without overshoot, the way unhurried human
/// movements do; slow-in/slow-out pacing of this kind also keeps moving objects the easiest to
/// follow (Dragicevic et al., CHI 2011).
pub(super) fn minimum_jerk(amount: f32) -> f32 {
    let amount = amount.clamp(0.0, 1.0);
    amount * amount * amount * (amount * (amount * 6.0 - 15.0) + 10.0)
}

/// How opaque a window must be under a layer of opacity `layer` for the two together to stay
/// exactly `whole` opaque, so what is behind them never shows more or less while one hands over
/// to the other. An opaque window stays opaque and the layer alone dissolves into it.
pub(super) fn beneath(whole: f32, layer: f32) -> f32 {
    if whole >= 1.0 {
        return 1.0;
    }
    (1.0 - (1.0 - whole) / (1.0 - layer)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layer_and_the_window_beneath_it_keep_the_opacity() {
        for whole in [0.3, 0.6, 0.9, 1.0] {
            for step in 0..=10 {
                let layer = whole * step as f32 / 10.0;
                let together = 1.0 - (1.0 - beneath(whole, layer)) * (1.0 - layer);
                assert!((together - whole).abs() < 1e-5, "{whole} under {layer} came to {together}");
            }
        }
        assert_eq!(beneath(0.6, 0.0), 0.6);
        assert_eq!(beneath(0.6, 0.6), 0.0);
        assert_eq!(beneath(1.0, 1.0), 1.0);
    }

    #[test]
    fn it_rests_at_both_ends_and_never_overshoots() {
        assert_eq!(minimum_jerk(0.0), 0.0);
        assert_eq!(minimum_jerk(1.0), 1.0);
        assert_eq!(minimum_jerk(2.0), 1.0);
        let steps: Vec<f32> = (0..=100).map(|step| minimum_jerk(step as f32 / 100.0)).collect();
        assert!(steps.windows(2).all(|pair| pair[1] >= pair[0]));
        assert!(minimum_jerk(0.01) < 1e-4 && 1.0 - minimum_jerk(0.99) < 1e-4);
    }

    #[test]
    fn its_speed_is_a_symmetric_bell_peaking_in_the_middle() {
        let speed = |at: f32| (minimum_jerk(at + 0.001) - minimum_jerk(at - 0.001)) / 0.002;
        assert!((speed(0.3) - speed(0.7)).abs() < 1e-2);
        assert!(speed(0.5) > speed(0.3) && (speed(0.5) - 1.875).abs() < 1e-2);
    }
}
