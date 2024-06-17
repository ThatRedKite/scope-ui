use std::f64::consts::PI;

use splines::{Key, Spline, Interpolation};
use regex::Regex;
use super::ValueUnitPair;

pub fn parse_unit(parse_string: &str) -> Result<ValueUnitPair, ()> {
    let match_pattern = Regex::new("(?<Value>[0-9]{1,3}|0.[0-9]{1,2})(?<Unit>mV|V|uV|s|ms|us)").unwrap();

    if let Some(captures) = match_pattern.captures(parse_string) {
        if let Ok(value) = (&captures["Value"]).parse::<f64>() {
            let unit_name = &captures["Unit"];
            let unit_mult: f64 = match unit_name {
                "s"|"V"   => {1.0},
                "ms"|"mV" => {1E3},
                "us"|"uV" => {1E6},
                _         => {0.0}
            };
            return Ok(ValueUnitPair{value:value, unit_mult:unit_mult, unit_name:unit_name.to_string()});
        }
    }
    Err(())
}


pub fn get_scale_units(condition_string: &String) -> Result<(ValueUnitPair,ValueUnitPair), ()> {
    let mut segments: Vec<&str> = condition_string.split(",").collect();
    let time_unit: ValueUnitPair;
    let voltage_unit: ValueUnitPair;
    println!("{}", segments.len());
    if segments.len() < 12 {
        return  Err(());
    }

    // empty channels will not have time or voltage information, but if one is present, we can assume the other is also present
    // this may not be the case if somehow the condition data was incorrectly written to using the Wi command
    
    if let Ok(result) = parse_unit(segments[3]) {
        time_unit = result;
        voltage_unit = parse_unit(segments[7]).unwrap();
    }
    else {
        return Err(());
    }
    

    Ok((time_unit,voltage_unit))
}

pub fn scale_time(x:usize, time_per_divison:f64, scale_factor: f64) -> f64 {
    // we have 10 divisions on the x axis and 1000 data points, so every division has a size of 100
    // so time/div becomes time/100
    ((time_per_divison / 100.0) * x as f64) *scale_factor
}

pub fn scale_waveform_data(waveform_data_raw: &Vec<u8>, voltage_per_division: f64, scale_factor: f64) -> Vec<f64> {
    // This scales the raw sample data bytes to the correct voltage and time as f64s
    let mut waveform_data_scaled: Vec<f64> = vec![0.0f64;1000];
    for (i, sample_data) in waveform_data_raw.iter().enumerate() {
        // since the zero line in the data is at 128, we need to subtract 128 to get to f64 0.0
        let voltage_y_corrected = -((*sample_data as f64) - 128.0);
        // since every y division has a size of 25, volts/div turns into volts/25
        let voltage_scaled = (voltage_y_corrected * (voltage_per_division / 25.0)) * scale_factor;
        // [time,voltage]
        waveform_data_scaled[i] = voltage_scaled;
    }
    waveform_data_scaled
}

pub fn unit_scale(samples: &Vec<f64>, voltage_unit: &ValueUnitPair) -> Vec<f64> {
    let scaled_samples = samples.iter().map(|sample| {sample * voltage_unit.unit_mult}).collect();
    scaled_samples
}  


pub fn cosine_interpolate_samples(samples: &Vec<f64>, num_samples: usize, time_per_divison:f64, step: usize) -> Vec<f64> {
    // Interpolates samples to n samples using Linear interpolation
    let mut keys: Vec<Key<f64,f64>> = Vec::with_capacity(samples.len() / step);
    let mut new_values:Vec<f64> = Vec::with_capacity(num_samples);
    
    for i in (0..samples.len()).step_by(step) {
        keys.push(Key::new(scale_time(i + 1, time_per_divison, 1.0),samples[i], Interpolation::Cosine));
    }

    // create a spline from the keys we got from the samples
    let spline = Spline::from_vec(keys);
    for i in 4..num_samples-4 {
        let x = scale_time(i, time_per_divison, 1.0)/(num_samples as f64 / 1000.0);
        if let Some(y_interpolated) = spline.clamped_sample(x) {
            new_values.push(y_interpolated);
        }
    }

    // if the number of samples is lower than requested, repeat the last sample until the number is correct
    if num_samples > new_values.len() {
        for _ in 0..(num_samples-new_values.len()) {
            let last_sample = new_values.last().unwrap();
            new_values.push(*last_sample);
        }
    }

    new_values

}


pub fn catmull_rom_interpolate_samples(samples: &Vec<f64>, num_samples: usize, time_per_divison:f64, step: usize) -> Vec<f64> {
    // Interpolates samples to n samples using Catmull-Rom splines
    let mut keys: Vec<Key<f64,f64>> = Vec::with_capacity(samples.len() / step);
    let mut new_values:Vec<f64> = Vec::with_capacity(num_samples);
    
    for i in (0..samples.len()).step_by(step) {
        keys.push(Key::new(scale_time(i + 1, time_per_divison, 1.0),samples[i], Interpolation::CatmullRom));
    }

    // catmull rom requires 4 keys, that won't work near the beginning or the end, meaning that we effectively "lose" some samples, idk

    // create a spline from the keys we got from the samples
    let spline = Spline::from_vec(keys);
    for i in 4..num_samples-4 {
        let x = scale_time(i, time_per_divison, 1.0)/(num_samples as f64 / 1000.0);
        if let Some(y_interpolated) = spline.clamped_sample(x) {
            new_values.push(y_interpolated);
        }
    }

    // if the number of samples is lower than requested, repeat the last sample until the number is correct
    if num_samples > new_values.len() {
        for _ in 0..(num_samples-new_values.len()) {
            let last_sample = new_values.last().unwrap();
            new_values.push(*last_sample);
        }
    }

    new_values
}

pub fn bezier2_interpolate_samples(samples: &Vec<f64>, num_samples: usize, time_per_divison:f64, step: usize) -> Vec<f64> {
    // Interpolates samples to n samples using Catmull-Rom splines
    let mut keys: Vec<Key<f64,f64>> = Vec::with_capacity(samples.len() / step);
    let mut new_values:Vec<f64> = Vec::with_capacity(num_samples);
    
    for i in (0..samples.len()-1).step_by(step) {
        let lo = samples[i];
        let avg_point = (lo + samples[i+1]) / 2.0;
        keys.push(Key::new(scale_time(i, time_per_divison, 1.0), lo, Interpolation::Bezier(avg_point)));
    }

    // catmull rom requires 4 keys, that won't work near the beginning or the end, meaning that we effectively "lose" some samples, idk

    // create a spline from the keys we got from the samples
    let spline = Spline::from_vec(keys);
    for i in 4..num_samples-4 {
        let x = scale_time(i, time_per_divison, 1.0)/(num_samples as f64 / 1000.0);
        if let Some(y_interpolated) = spline.clamped_sample(x) {
            new_values.push(y_interpolated);
        }
    }

    // if the number of samples is lower than requested, repeat the last sample until the number is correct
    if num_samples > new_values.len() {
        for _ in 0..(num_samples-new_values.len()) {
            let last_sample = new_values.last().unwrap();
            new_values.push(*last_sample);
        }
    }

    new_values

}

pub fn bezier_interpolate_samples(samples: &Vec<f64>, num_samples: usize, time_per_divison:f64, step: usize) -> Vec<f64> {
    // Interpolates samples to n samples using Catmull-Rom splines
    let mut keys: Vec<Key<f64,f64>> = Vec::new();
    let mut new_values:Vec<f64> = Vec::with_capacity(num_samples);
    
    for i in (0..samples.len()-1).step_by(step+1) {
        keys.push(Key::new(scale_time(i, time_per_divison, 1.0), samples[i], Interpolation::Bezier(samples[i+1])));
    }

    // catmull rom requires 4 keys, that won't work near the beginning or the end, meaning that we effectively "lose" some samples, idk

    // create a spline from the keys we got from the samples
    let spline = Spline::from_vec(keys);
    for i in 4..num_samples-4 {
        let x = scale_time(i, time_per_divison, 1.0)/(num_samples as f64 / 1000.0);
        if let Some(y_interpolated) = spline.clamped_sample(x) {
            new_values.push(y_interpolated);
        }
    }

    // if the number of samples is lower than requested, repeat the last sample until the number is correct
    if num_samples > new_values.len() {
        for _ in 0..(num_samples-new_values.len()) {
            let last_sample = new_values.last().unwrap();
            new_values.push(*last_sample);
        }
    }

    new_values

}

pub fn linear_interpolate_samples(samples: &Vec<f64>, num_samples: usize, time_per_divison:f64, step: usize) -> Vec<f64> {
    // Interpolates samples to n samples using Linear interpolation
    let mut keys: Vec<Key<f64,f64>> = Vec::new();
    let mut new_values:Vec<f64> = Vec::with_capacity(num_samples);
    

    for i in (0..samples.len()).step_by(step) {
        let data = samples[i];
        keys.push(Key::new(scale_time(i, time_per_divison, 1.0), data, Interpolation::Linear));
    }


    // create a spline from the keys we got from the samples
    let spline = Spline::from_vec(keys);
    for i in 4..num_samples-4 {
        let x = scale_time(i, time_per_divison, 1.0)/(num_samples as f64 / 1000.0);
        if let Some(y_interpolated) = spline.clamped_sample(x) {
            new_values.push(y_interpolated);
        }
    }

    // if the number of samples is lower than requested, repeat the last sample until the number is correct
    if num_samples > new_values.len() {
        for _ in 0..(num_samples-new_values.len()) {
            let last_sample = new_values.last().unwrap();
            new_values.push(*last_sample);
        }
    }

    new_values

}

pub fn moving_average_filter(samples: &Vec<f64>, window_size: usize) -> Vec<f64> {
    let mut filtered_samples: Vec<f64> = Vec::with_capacity(samples.len());
    // calculate the initial sum
    let mut sum: f64 = 0.0;

    for (i, sample) in samples.iter().enumerate() {
        // i = 18
        // window_size = 6
        // len = 32


        // 18 >= 6 (false)
        if i >= window_size {
            sum -= samples[i - window_size];
        }

        // 18 + 6 = 22; 22 <= 32 (true)
        if i + window_size <= samples.len() {
            // do this
            sum += sample;
        }
        
        // 18 >= 6 - 1 = 5; 18 >= 5 (true)
        if i >= window_size - 1 {
            filtered_samples.push(sum / window_size as f64);
        }
    }
    while filtered_samples.len() < samples.len() {
        filtered_samples.insert(0,filtered_samples[0]);
        filtered_samples.push(filtered_samples[filtered_samples.len() - 1]);
    }
    
    filtered_samples
}

pub fn make_rectangle(voltage_per_division:f64, amplitude:f64, time_per_division:f64, period:f64) -> Vec<f64> {
    let mut new_samples: Vec<f64> = Vec::with_capacity(1000);
    for x in 1..1001{
        let square = ((x as f64 / (32.0 * period)).sin().floor()) * voltage_per_division * amplitude * 2.0;
        new_samples.push(square);
    }
    new_samples
}

// (sin(pi (x - i))/pi (x - i))*y

pub fn test_filter(num_samples:usize, step: f64, time_per_divison:f64, voltage_per_division: f64) -> Vec<f64> {
    let mut new_samples: Vec<f64> = Vec::with_capacity(num_samples);
    for x in 1..1001{
        let square = ((x as f64 / (32.0 * step)).sin().floor()) * voltage_per_division * 2.0;
        new_samples.push(square);
    }
    new_samples
}