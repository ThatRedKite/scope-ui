#![allow(dead_code, unused_mut)]
use std::borrow::BorrowMut;
use std::io::{BufRead, BufReader, Write};
use std::{io::Read, time::Duration};
use std::thread::sleep;
use serialport::{self,StopBits, TTYPort};

pub mod channels;
pub mod errors;
pub mod condition;
pub mod processing;

use channels::Channel;
use errors::OscilloscopeError;


fn make_ri_command(channel: Channel, start_address:u32, end_address:u32) -> String {
    //construct command from string and parameters
    return format!("R{}({:04},{:04},B)\r", channel as u8, start_address, end_address);
}

fn make_ro_command(channel: Channel) -> String {
    return format!("Ro({})\r", channel as u8);
}

#[derive(Clone, Default)]
pub struct ValueUnitPair {
    pub value: f64,
    pub unit_mult: f64,
    pub unit_name: String
}

pub struct OscilloscopeCapture {
    pub port:               TTYPort,
    response_data:      Vec<u8>,
    command_buffer:     Vec<u8>,
    cond_string:        String
}

impl OscilloscopeCapture {
    pub fn new(port_name: &str, baud_rate: u32, stopbits: StopBits) -> OscilloscopeCapture {
        let mut port: TTYPort = serialport::new(port_name, baud_rate)
        .stop_bits(stopbits)
        .parity(serialport::Parity::None)
        .flow_control(serialport::FlowControl::None)
        .data_bits(serialport::DataBits::Eight)
        .timeout(Duration::from_millis(2000))
        .open_native()
        .expect("Failed to open port");
        
        let mut response_data: Vec<u8>          =    Vec::with_capacity(1015);
        let mut command_buffer: Vec<u8>         =    Vec::with_capacity(32);
        let mut cond_string: String             =    String::new();

        return OscilloscopeCapture{port,response_data: response_data,command_buffer, cond_string};
    }

    fn make_command(self: &mut Self, command: String) {
        //let command_bytes:Vec<u8> = Vec::new();
        self.command_buffer.clear();
        for byte in command.as_bytes() {
            self.command_buffer.push(*byte)
        }
    } 

    fn eval_response(self: &mut Self) -> bool {
        let mut response_buffer: Vec<u8> = Vec::new();
        let mut buffy = BufReader::new(self.port.borrow_mut());
        sleep(Duration::from_millis(10));
        if let Ok(num) = buffy.read_until(0x0D, &mut response_buffer) {
            //println!("Response: {}", char::from(response_buffer[0]));
            return num == 2 && response_buffer[0] == 0x41;
        }
        false 
    }

    pub fn send_s1(self: &mut Self) -> Result<(), OscilloscopeError> {
        // write S1 command to command buffer
        self.make_command(String::from("S1\r"));
        if let Ok(_) =  self.port.write(&self.command_buffer) {
            // clear command buffer
            self.command_buffer.clear();
            // evaluate response
            if self.eval_response() {return Ok(());}
            else{Err(OscilloscopeError::S1Failure)}
        }
        else {
            self.command_buffer.clear();
            Err(OscilloscopeError::S1Failure)
        }
    }
    
    pub fn send_ro(self: &mut Self, channel: Channel) -> Result<(), OscilloscopeError> {
        // clear all relevant buffers
        self.command_buffer.clear();
        self.cond_string.clear();

        // make Ro(channel) command
        self.make_command(make_ro_command(channel));

        if let Ok(_) = self.port.write_all(&self.command_buffer) {
            sleep(Duration::from_millis(1000));
            let mut reader = BufReader::new(&mut self.port);
            let mut local_buffer = Vec::<u8>::with_capacity(68);

            if let Ok(num) = reader.read_until(0x0D, &mut local_buffer) {
                // ensure that the received data has the required length
                println!("{}", num);
                if num == 68 {
                    // convert the result into a string and store it in self.cond_string
                    if let Ok(cond_string) = String::from_utf8(local_buffer) {
                        self.cond_string = cond_string;
                        // clear command buffer
                        self.command_buffer.clear();
                        return Ok(());
                    }
                    // throw an error if it fails to get the string like when a channel has no information
                    else {
                        return Err(OscilloscopeError::RoError);
                    }
                    

                }

                else {
                    return  Err(OscilloscopeError::S1Failure);
                }
            }

        } 
        Err(OscilloscopeError::RiError)
    }

    pub fn send_ri(self: &mut Self, channel: Channel, start_address:u32, end_address:u32) -> Result<(), OscilloscopeError> {
        self.command_buffer.clear();
        //construct Ri command and write it to the command buffer
        self.make_command(make_ri_command(channel, start_address, end_address));
        // write command
        if let Ok(_) = self.port.write_all(&self.command_buffer) {
            sleep(Duration::from_millis(750));
            // clear waveform buffer
            self.response_data.clear();
            // read until we get a CR or time out
            let mut reader = BufReader::new(&mut self.port);
            match reader.read_until(0x0D, &mut self.response_data) {
                Ok(num) => {
                    if num == (end_address - start_address) as usize + 15 {
                        self.command_buffer.clear();
                        self.response_data.pop();
                        return Ok(());
                    }},
                Err(_e) => {
                    return Err(OscilloscopeError::RiError);
                }
            }
        }
        Err(OscilloscopeError::RiError)
    }
    
    pub fn s1_recover(self: &mut Self) {
        //eprintln!("S1 Error");
        sleep(Duration::from_secs(1));
        let mut kill_buf:Vec<u8> = Vec::new();
        let _ = self.port.read_to_end(&mut kill_buf);
        //let _ = self.port.clear(serialport::ClearBuffer::All);
        self.command_buffer.clear();
    }
    

    #[allow(unused_assignments)]
    pub fn get_waveform_data(self: &mut Self, channel: Channel, scale: f64) -> Result<(Vec<f64>, ValueUnitPair, ValueUnitPair), OscilloscopeError> {
        // send s1

        // initialize 
        let mut waveform_data: Vec<f64> = Vec::new();

        // try up to 10 times to get a successful S1
        self.send_s1()?;

        
        // sleep a bit
        sleep(Duration::from_millis(500));
        println!("S1 Successful");

        // now we need to get the condition data to accurately scale the data 
        
        // first, assign self.cond_string by sending the Ro(channel) command which returns a string containing all the relevant information
        self.send_ro(channel)?;

        // now we need to parse the string to get the time and voltage scales
        if let Ok((time_unit, voltage_unit)) = processing::get_scale_units(&self.cond_string) {
            // it's time to actually get the waveform data, this is a _very_ unrealiable process at "high" baudrates like 9600 so it's likely to fail
            // wait a bit because the scope is pretty slow

            sleep(Duration::from_millis(250));
            println!("Ro Successful");

            self.send_ri(channel, 0, 1000)?;

            println!("Ri Successful");
            // now we need to scale the raw waveform data correctly and turn it into a series of f64 points
            waveform_data = processing::scale_waveform_data(&self.response_data[14..1000].to_vec(), voltage_unit.value, scale);
            waveform_data = processing::unit_scale(&waveform_data, &voltage_unit);
            // now we could interpolate the data or we could do it in real time
            //waveform_data = scaling::spline_interpolate_samples(&waveform_data, num_samples, time_unit.value);

            Ok((waveform_data, time_unit, voltage_unit))
        }
        else {
            return Err(OscilloscopeError::RoError);
        }
    }
}
