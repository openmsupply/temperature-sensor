//! # Temperature Sensor
//!
//! `temperature_sensor` is a collection of utilities to parse data files
//! generated from temperature sensors and return details of the sensor,
//! its breach configurations, recorded breaches and temperature logs in
//! a standard format.
//!
//! It has been implemented for use in our open mSupply LMIS software, which
//! is being rewritten in Rust <https://msupply.foundation/projects/omsupply>.
//!
//! So far it only supports Berlinger Fridge-tag and Q-tag USB sensors
//! <https://www.berlinger.com/cold-chain-management> but it is hoped to extend
//! it to other sensor types in future.
//!
//! (1) Berlinger Fridge-tags without logging e.g. Fridge-tag 2 or Fridge-tag UL:
//!
//! Temperature logs are only recorded for the max & min temperature each day, and
//! only cumulative breaches (from midnight to midnight) are recorded, where:
//!     Alarm=0 => cold (cumulative) breach
//!     Alarm=1 => hot (cumulative) breach
//!
//! Note that the sensor doesn't record the start or end of the breach, only the time
//! when the breach was triggered and the total duration of the breach
//! => we work out the breach start time by subtracting the breach config duration
//! from the breach trigger time (or midnight if that is later) and we work out
//! the breach end time by adding the total breach duration to the breach start time
//! (or midnight if that is earlier).
//!
//! Obviously, these calculations will only be correct if the breach is continuous i.e.
//! there are no gaps when the temperature is not breaching, but it's the best that can
//! be done with the limited data available.
//!
//! (2) Berlinger Fridge-tags with logging e.g. Fridge-tag 2L:
//!
//! These record breaches in the same way and have the same limitations, but they also
//! record full temperature logs (usually every 5 minutes) => it would be possible to
//! "correct" the calculated breach start and end times by processing the temperature
//! logs for the same day by applying the following 3 sets of rules:
//!
//! (a) expand the start & end times based on the first & last breaching temperature logs
//! of the day:
//!    - if the first breaching temperature log is before the calculated breach start time,
//!      then set the breach start time to the temperature log time.
//!    - if the last breaching temperature log is after the calculated breach end time,
//!      then set the breach end time to the temperature log time.
//!
//! (b) take account of the sensor log interval at the start & end of the day:
//!    - if the first breaching temperature log is within the sensor log interval of
//!      midnight, then set the breach start time to midnight.
//!    - if the last breaching temperature log is within the sensor log interval of
//!      midnight, then set the breach end time to midnight.
//!
//! (c) correct for other discrepancies when it's a non-continuous breach:
//!    - if the first breaching temperature log is more than the sensor log interval
//!      after the calculated breach start time, then set the breach start time to the
//!      temperature log time.
//!    - if the last breaching temperature log is more than the sensor log interval
//!      before the calculated breach end time, then set the breach end time to the
//!      temperature log time.
//!
//! As we have the temperature logs, we can use these to detect consecutive breaches,
//! assuming that the same breach configurations apply (i.e. the same temperature &
//! duration thresholds). Unlike cumulative breaches, which are only midnight to midnight,
//! consecutive breaches have the potential to cover more than one day if they are ongoing
//! at midnight, and of course you can have multiple consecutive breaches per day.
//!
//! (3) Berlinger Q-tags (all with logging?) e.g. Q-tag CLm doc LR:
//!
//! These can have up to 5 breach configurations, each having one of the following types:
//!     Alarm type=1 => cold consecutive breach
//!     Alarm type=2 => hot consecutive breach
//!     Alarm type=3 => cold cumulative breach
//!     Alarm type=4 => hot cumulative breach
//!
//! Q-tags record the start and end time (and duration) of consecutive breaches,
//! although they only record the start time of cumulative breaches, so the end time
//! still needs to be calculated in the same way as for Fridge-tags i.e. by adding the
//! breach duration to the start time. For non-continuous cumulative breaches, the
//! true end time can be calculated from the last breaching temperature log of the day.
//!

pub mod berlinger;
pub mod common;

use std::fs::File;
use std::io::Write;

pub use crate::common::{
    BreachType, Sensor, SensorType, TemperatureBreach, TemperatureBreachConfig, TemperatureLog,
};

use chrono::{Duration, Local, NaiveDateTime};

/// Returns some made-up example temperature sensor data, for use in automated tests.
pub fn sample_sensor() -> Sensor {
    let config_cold_consecutive = TemperatureBreachConfig {
        breach_type: BreachType::ColdConsecutive,
        maximum_temperature: 100.0,
        minimum_temperature: 2.0,
        duration: Duration::seconds(240),
    };

    let config_hot_consecutive = TemperatureBreachConfig {
        breach_type: BreachType::HotConsecutive,
        maximum_temperature: 8.0,
        minimum_temperature: -273.0,
        duration: Duration::seconds(300),
    };

    let temperature_values = vec![
        3.5, 4.0, 5.0, 7.5, // ok
        8.8, 9.2, 8.7, 9.1, 8.4, 8.2, 8.1, //hot
        7.9, 3.2, // ok
        1.2, 1.3, 0.4, -0.2, 0.7, // cold
        2.5, // ok
    ];

    let mut temperature_timestamp =
        NaiveDateTime::parse_from_str("2023-05-23 13:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let interval = Duration::minutes(1);
    let hot_start_timestamp = temperature_timestamp + interval * 4;
    let hot_end_timestamp = temperature_timestamp + interval * 10;
    let hot_duration = hot_end_timestamp - hot_start_timestamp; //interval*(10-4);
    let cold_start_timestamp = temperature_timestamp + interval * 13;
    let cold_end_timestamp = temperature_timestamp + interval * 17;
    let cold_duration = cold_end_timestamp - cold_start_timestamp; //interval*(17-13);

    let temperature_iterator = temperature_values.iter();
    let mut temperature_logs: Vec<TemperatureLog> = Vec::new();

    for temperature_value in temperature_iterator {
        temperature_logs.push(TemperatureLog {
            temperature: *temperature_value,
            timestamp: temperature_timestamp,
        });
        temperature_timestamp = temperature_timestamp + interval;
    }

    let breach_cold_consecutive = TemperatureBreach {
        breach_type: BreachType::ColdConsecutive,
        start_timestamp: cold_start_timestamp,
        end_timestamp: cold_end_timestamp,
        duration: cold_duration,
        acknowledged: false,
    };

    let breach_hot_consecutive = TemperatureBreach {
        breach_type: BreachType::HotConsecutive,
        start_timestamp: hot_start_timestamp,
        end_timestamp: hot_end_timestamp,
        duration: hot_duration,
        acknowledged: false,
    };

    let sensor = Sensor {
        sensor_type: SensorType::Berlinger,
        serial: String::from("reg 1234"),
        name: String::from("Berlinger 1"),
        last_connected_timestamp: Some(temperature_timestamp),
        log_interval: Some(interval),
        breaches: Some(vec![breach_hot_consecutive, breach_cold_consecutive]),
        configs: Some(vec![config_cold_consecutive, config_hot_consecutive]),
        logs: Some(temperature_logs),
    };

    sensor
}

/// Returns all sensors found from currently mounted USB drives up to 8GB capacity
/// (-> any USB drive containing sensor files if you don't have a physical sensor).
/// For Berlinger sensors, it expects to find a serial_xxxxx.txt file in the root folder
/// together with a matching PDF file (USB drives can have multiple pairs of files).
pub fn read_connected_sensors() -> Result<Vec<Sensor>, String> {
    if let Some(sensor_array) = berlinger::read_sensors_from_usb() {
        Ok(sensor_array)
    } else {
        Err("No sensors found".to_string())
    }
}

/// Returns all the serials found from currently mounted USB drives up to 8GB capacity
/// (-> any USB drive containing sensor files if you don't have a physical sensor).
/// For Berlinger sensors, it expects to find a serial_xxxxx.txt file in the root folder
/// together with a matching PDF file (USB drives can have multiple pairs of files).
pub fn read_connected_serials() -> Result<Vec<String>, String> {
    if let Some(sensor_serials) = berlinger::read_sensor_serials() {
        log::info!("Serials found: {:?}", sensor_serials);
        Ok(sensor_serials)
    } else {
        Err("No sensors found".to_string())
    }
}

/// Reads sensor data from the specified sensor txt file.
pub fn read_sensor_file(file_path: &str) -> Result<Sensor, String> {
    if let Some(sensor) = berlinger::read_sensor_from_file(&file_path) {
        if cfg!(debug_assertions) {
            // Generate output file for debugging/reference
            let output_path = "sensor_".to_owned() + &sensor.serial + "_output.txt";
            if let Some(mut output) = File::create(&output_path).ok() {
                if write!(output, "{}", format!("{:?}\n\n", sensor)).is_ok() {
                    log::info!("Output: {}", &output_path)
                }
            }
        }

        Ok(sensor)
    } else {
        Err("Sensor file not found".to_string())
    }
}

/// Reads sensor data from the contents of a txt file, by writing the
/// contents to a local txt file and reading that.
pub fn parse_sensor(file_contents: &str) -> Result<Sensor, String> {
    let file_path = format!("sensor_input_{}.txt", Local::now().timestamp());
    if let Some(mut output) = File::create(&file_path).ok() {
        if write!(output, "{}", file_contents).is_ok() {
            log::info!("Reading sensor from: {}", &file_path);
            return read_sensor_file(&file_path);
        }
    }
    Err("Sensor file not created".to_string())
}

/// Reads sensor data from USB for the txt file corresponding to the specified serial.
/// Note that the serial is expected to match the corresponding serial field inside
/// the txt file.
pub fn read_sensor(serial: &str) -> Result<Sensor, String> {
    if let Some(sensor_array) = berlinger::read_sensors_from_usb() {
        for sensor in sensor_array {
            if sensor.serial == serial.to_string() {
                log::info!("Found sensor: {}", serial);

                if cfg!(debug_assertions) {
                    // Generate output file for debugging/reference
                    let output_path = "sensor_".to_owned() + &sensor.serial + "_output.txt";
                    if let Some(mut output) = File::create(&output_path).ok() {
                        if write!(output, "{}", format!("{:?}\n\n", sensor)).is_ok() {
                            log::info!("Output: {}", &output_path)
                        }
                    }
                }

                return Ok(sensor);
            }
        }
    }

    return Err("Sensor not found".to_string());
}

/// Applies optional start/end timestamps to the breaches and temperature logs
/// of the specified sensor e.g. to include only data since the last time the
/// sensor was read (or from the start of the last recorded breach if it was
/// ongoing at the time of the last sensor read).
///
/// Temperature logs are filtered out if they are either before the start timestamp
/// or after the end timestamp.
///
/// Breaches are filtered out if they are entirely before the start timestamp or after
/// the end timestamp i.e. keep if any part of the breach is between the start timestamp
/// and the end timestamp.
///
/// Note that the difference between the start and end breach timestamps is only
/// the same as the breach duration for consecutive breaches which start and end
/// within the specified interval.
pub fn filter_sensor(
    mut sensor: Sensor,
    start_timestamp: Option<NaiveDateTime>,
    end_timestamp: Option<NaiveDateTime>,
) -> Sensor {
    if let Some(start) = start_timestamp {
        let mut filtered_logs: Vec<TemperatureLog> = Vec::new();
        match sensor.logs {
            Some(logs) => {
                for log in logs {
                    if log.timestamp >= start {
                        filtered_logs.push(log);
                    }
                }
                if filtered_logs.len() > 0 {
                    sensor.logs = Some(filtered_logs);
                } else {
                    sensor.logs = None;
                }
            }
            None => {}
        };
        let mut filtered_breaches: Vec<TemperatureBreach> = Vec::new();
        match sensor.breaches {
            Some(breaches) => {
                for breach in breaches {
                    if breach.start_timestamp >= start {
                        // keep if start of breach is after start timestamp
                        filtered_breaches.push(breach);
                    } else if breach.end_timestamp >= start {
                        // if start of breach is before start timestamp
                        filtered_breaches.push(breach); // keep if end of breach is after start timestamp
                    }
                }
                if filtered_breaches.len() > 0 {
                    sensor.breaches = Some(filtered_breaches);
                } else {
                    sensor.breaches = None;
                }
            }
            None => {}
        };
    }

    if let Some(end) = end_timestamp {
        let mut filtered_logs: Vec<TemperatureLog> = Vec::new();
        match sensor.logs {
            Some(logs) => {
                for log in logs {
                    if log.timestamp <= end {
                        filtered_logs.push(log);
                    }
                }
                if filtered_logs.len() > 0 {
                    sensor.logs = Some(filtered_logs);
                } else {
                    sensor.logs = None;
                }
            }
            None => {}
        };
        let mut filtered_breaches: Vec<TemperatureBreach> = Vec::new();
        match sensor.breaches {
            Some(breaches) => {
                for breach in breaches {
                    if breach.end_timestamp <= end {
                        // keep if end of breach is before end timestamp
                        filtered_breaches.push(breach);
                    } else if breach.start_timestamp <= end {
                        // if end of breach is after end timestamp
                        filtered_breaches.push(breach); // keep if start of breach is before end timestamp
                    }
                }
                if filtered_breaches.len() > 0 {
                    sensor.breaches = Some(filtered_breaches);
                } else {
                    sensor.breaches = None;
                }
            }
            None => {}
        };
    }

    if cfg!(debug_assertions) {
        // Generate output file for debugging/reference
        let output_path = "sensor_".to_owned() + &sensor.serial + "_filtered_output.txt";
        if let Some(mut output) = File::create(&output_path).ok() {
            if write!(output, "{}", format!("{:?}\n\n", sensor)).is_ok() {
                log::info!(
                    "Filtered output from {:?} - {:?} to: {}",
                    start_timestamp,
                    end_timestamp,
                    &output_path
                );
            }
        }
    }

    return sensor;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_core() {
        let sensor = sample_sensor();
        assert_eq!(sensor.serial, "reg 1234");
        assert!(sensor.breaches.is_some());
        assert!(sensor.logs.is_some());
        assert!(sensor.configs.is_some());
    }

    #[test]
    fn test_sample_breach() {
        let sensor = sample_sensor();
        let start_timestamp =
            NaiveDateTime::parse_from_str("2023-05-23 13:04:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let end_timestamp =
            NaiveDateTime::parse_from_str("2023-05-23 13:17:00", "%Y-%m-%d %H:%M:%S").unwrap();
        if let Some(breaches) = sensor.breaches {
            assert_eq!(breaches[0].start_timestamp, start_timestamp); // start of hot breach
            assert_eq!(breaches[1].end_timestamp, end_timestamp); // end of cold breach
        }
    }

    #[test]
    fn test_sample_log() {
        let sensor = sample_sensor();
        let start_timestamp =
            NaiveDateTime::parse_from_str("2023-05-23 13:04:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let end_timestamp =
            NaiveDateTime::parse_from_str("2023-05-23 13:17:00", "%Y-%m-%d %H:%M:%S").unwrap();
        if let Some(logs) = sensor.logs {
            assert_eq!(logs[4].timestamp, start_timestamp); // start of hot breach
            assert_eq!(logs[17].timestamp, end_timestamp); // end of cold breach
        }
    }

    #[test]
    fn test_sample_filter_breach() {
        let start_timestamp =
            NaiveDateTime::parse_from_str("2023-05-23 13:07:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let end_timestamp =
            NaiveDateTime::parse_from_str("2023-05-23 13:15:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let sensor = filter_sensor(sample_sensor(), Some(start_timestamp), Some(end_timestamp));
        if let Some(breaches) = sensor.breaches {
            assert_eq!(breaches[0].start_timestamp, start_timestamp); // start of hot breach changed
            assert_eq!(breaches[1].end_timestamp, end_timestamp); // end of cold breach changed
        }
    }

    #[test]
    fn test_sample_filter_log() {
        let start_timestamp =
            NaiveDateTime::parse_from_str("2023-05-23 13:07:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let end_timestamp =
            NaiveDateTime::parse_from_str("2023-05-23 13:15:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let sensor = filter_sensor(sample_sensor(), Some(start_timestamp), Some(end_timestamp));
        if let Some(logs) = sensor.logs {
            assert_eq!(logs[0].timestamp, start_timestamp); // start of hot breach changed
            assert_eq!(logs[8].timestamp, end_timestamp); // end of cold breach changed
        }
    }
}
