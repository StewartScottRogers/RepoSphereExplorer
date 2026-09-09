//! Regenerates `samples/hdf5/sensor-readings.h5`.
//!
//! Run with:
//! `cargo run -p plugin-hdf5 --example generate_fixture -- samples/hdf5/sensor-readings.h5`

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: generate_fixture <output path>");

    let file = hdf5_metno::File::create(path).expect("create output file");
    file.new_dataset::<i32>()
        .create("run_id")
        .expect("create scalar dataset");

    let experiment = file
        .create_group("experiment")
        .expect("create experiment group");
    experiment
        .new_dataset::<i64>()
        .shape(6)
        .create("timestamps")
        .expect("create 1-D dataset");

    let sensors = experiment
        .create_group("sensors")
        .expect("create nested sensors group");
    sensors
        .new_dataset::<f64>()
        .shape((3, 4))
        .create("temperature")
        .expect("create 2-D dataset");
    sensors
        .new_dataset::<i32>()
        .shape(4)
        .create("fault_counts")
        .expect("create second 1-D dataset");
}
