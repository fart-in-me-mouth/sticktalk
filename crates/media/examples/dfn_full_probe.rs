//! Explicit FILE-only full-model compatibility probe. No embedded model changes.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: dfn_full_probe <DeepFilterNet3_onnx.tar.gz>")?;
    let params = df::tract::DfParams::new(path.into())?;
    let start = std::time::Instant::now();
    let mut model = match df::tract::DfTract::new(params, &df::tract::RuntimeParams::default()) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("FULL DeepFilterNet3 initialization failed: {e:#}");
            return Err(e.into());
        }
    };
    println!(
        "FULL DeepFilterNet3 initialized in {:?}: {} Hz, hop {}",
        start.elapsed(),
        model.sr,
        model.hop_size
    );
    let input: Vec<f32> = (0..model.hop_size)
        .map(|i| (i as f32 * 0.03).sin() * 0.1)
        .collect();
    let mut output = vec![0.; model.hop_size];
    let start = std::time::Instant::now();
    for _ in 0..100 {
        model.process(
            ndarray::ArrayView2::from_shape((1, input.len()), &input)?,
            ndarray::ArrayViewMut2::from_shape((1, output.len()), &mut output)?,
        )?;
    }
    println!("FULL model 100-hop compute: {:?}", start.elapsed());
    Ok(())
}
