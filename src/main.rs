use imgui::*;
use mint::Vector4;
use serialport::StopBits;
use std::{sync::{mpsc, Arc, Mutex}, thread::{self, sleep}, time::Duration, vec};
use std::sync::mpsc::{Sender,Receiver};

mod support;
mod os3000;
use os3000::{channels::Channel, processing::{self, bezier2_interpolate_samples, bezier_interpolate_samples, catmull_rom_interpolate_samples, cosine_interpolate_samples, linear_interpolate_samples}, OscilloscopeCapture, ValueUnitPair};
use os3000::errors::OscilloscopeError;
#[derive(Clone)]
struct CaptureConfig {
    do_capture: bool,
    open_port: bool,
    two_stopbits: bool,
    do_s1: bool,
    do_ro: bool,
    do_ri: bool,
    port_name: String,
    channel: Channel,
    baudrate: u32,
}

#[allow(dead_code)]
enum ScopeStatus {
    Idle,
    S1,
    S1Fail,
    S1Retry,
    S1Success,
    Ro,
    RoFail,
    RoSuccess,
    Ri,
    RiFail,
    RiRetry,
    RiSuccess,
    UnknownError
}

struct ScopeResponse {
    s1_result: bool,
    capture_conditions: String,
    waveform_data: Vec<f64>,
    time_per_div: ValueUnitPair,
    voltage_per_div: ValueUnitPair
}

const BAUDRATES: [&str; 6] = ["300", "600", "1200", "2400", "4800", "9600"];


#[doc = "Draws a trace on a window using the drawlist"]
fn draw_trace_lines(samples: &Vec<f64>, drawlist: &DrawListMut, c: ImColor32, start_index: usize,end_index: usize, offsets: (f32, f32), scales: (f32, f32),thickness:f32 ,ui: &Ui) {
    // all our samples are f64, imgui unfortunately only wants f32
    let mut last_point = [0.0, samples[start_index] as f32];
    let (win_x, win_y) = ui.window_pos().into();
    for i in start_index..end_index {
        //               window x coordinate + current index - start index       
        let new_x = win_x + (i - start_index) as f32 * scales.0;
        //                sample data              center trace at window cetner                   y scale            y offset
        let new_y = (((samples[i] as f32 * 255.0 * scales.1) + (win_y + 5.0) + (ui.window_size()[1]/2.0)) ) + offsets.1;
        drawlist.add_line(last_point, [new_x, new_y], c).thickness(thickness).build();
        last_point = [new_x, new_y];
    }
}

#[doc = "Draws a trace on a window using the drawlist"]
fn draw_trace_dots(samples: &Vec<f64>, drawlist: &DrawListMut, c: ImColor32, start_index: usize,end_index: usize, offsets: (f32, f32), scales: (f32, f32),thickness:f32 ,ui: &Ui) {
    // all our samples are f64, imgui unfortunately only wants f32
    let (win_x, win_y) = ui.window_pos().into();
    
    for i in start_index..end_index {
        //               window x coordinate + current index - start index       
        let new_x = win_x + (i - start_index) as f32 * scales.0;
        //                sample data              center trace at window cetner        y scale                       y offset
        let new_y = (((samples[i] as f32 * 255.0 * scales.1) + (win_y + 5.0) + (ui.window_size()[1]/2.0)) ) + offsets.1;
        drawlist.add_circle([new_x, new_y], thickness, c).filled(true).build();
    }
}

#[doc = "Update the start index for dragging the waveform on screen"]
fn update_start_index(index_end:usize, index_start: &mut usize, ui: &Ui, x_scale: f32) {
    let mouse_drag_delta_x = ui.mouse_drag_delta()[0];
    let window_width = ui.window_size()[0];

        // change start index by dragging the cursor left or right
    if (mouse_drag_delta_x as usize) < index_end && index_end > *index_start && !ui.io().key_ctrl && ui.is_window_hovered() {
        let index_start_new = (*index_start as f32 + mouse_drag_delta_x / 10.0) as usize;
        
        // update index if drag_distance is smaller than index - window width
        if (index_start_new as isize) < index_end as isize - (window_width as isize) {
            *index_start = index_start_new;
        }
        // make it move slowly near the end of indexes
        else if *index_start as isize + 1 < (index_end as isize) - (window_width as isize) {
            *index_start += 1;
        }
    }

    // force index to 0 if window width equals number of points
    if index_end as isize - ((window_width.floor() * (x_scale / 2.0)) as isize) <= 0 {
        *index_start = 0;
    }

    // prevent start index from exceeding the end index
    if *index_start >= index_end {
        *index_start = index_end - 1;
    }
}

#[doc = "Draws a 5x4 grid"]
fn draw_grid_lines(line_color: ImColor32, y_offset: f32, ui: &Ui ,draw_list: &DrawListMut) {
    let (win_x, win_y) = ui.window_pos().into();
    let (window_width, window_height) = ui.window_size().into();

    // draw vertical lines 
    for i in 1..10 {
        let offset = (window_width / 10.0) * i as f32;
        if i != 5 {draw_list.add_line([win_x + offset, win_y + y_offset], [win_x + offset, win_y + window_height + y_offset], line_color).build();}
        // make center line thicker
        else {draw_list.add_line([win_x + offset, win_y + y_offset], [win_x + offset, win_y + window_height + y_offset], line_color).thickness(4.0).build();}
    }
    // draw horizontal_lines
    for i in 1..8 {
        let offset = (window_height / 8.0) * i as f32;
        if i != 4 {draw_list.add_line([win_x, win_y + offset + y_offset], [win_x + window_width, win_y + offset + y_offset], line_color).build();}
        // make center line thicker
        else {draw_list.add_line([win_x, win_y + offset + y_offset], [win_x + window_width, win_y + offset + y_offset], line_color).thickness(3.0).build();}
    }
}


#[allow(unused_mut)]
fn main() {
    let mut do_capture    : bool = false;
    let mut single_capture: bool = false;

    let mut port_string:      String = String::from("");
    let mut status_string = "Idle";

    let mut waveform_buffer: Vec<f64> = vec![0.0f64; 1000];
    let mut availible_ports: Vec<String> = Vec::new();

    let mut channel: Channel = Channel::DISPLAY1;
    let mut mode_radiobutton:u8 = 2;

    let mut x_scale: f32 = 1.0;
    let mut y_scale: f32 = 1.0;

    let mut y_offset: f32 = 0.0;
    let mut x_offset: usize = 0;

    let mut interpol_samples: usize = 1000;
    let mut interpol_step: usize = 2;
    let mut interpol2_samples: usize = waveform_buffer.len() * 2;
    let mut interpol2_step: usize = 1;
    let mut interpolation_method: u8 = 0;

    let mut avg_window_size: usize = 3;
    let mut max_window_size: usize = 1000;

    let mut time_per_div: ValueUnitPair = ValueUnitPair::default();
    let mut voltage_per_div: ValueUnitPair = ValueUnitPair::default();

    let mut index_start: usize = 0;

    let mut draw_average = false;
    let mut draw_main_trace = true;
    let mut draw_grid = true;
    let mut snap_to_trace = false;
    let mut draw_dots = false;

    let mut trace_thickness: f32 = 2.0;
    let mut avg_thickness: f32 = 2.0;

    let mut trace_color = Vector4::from([1.0,0.1,0.1,1.0]);
    let mut avg_color = Vector4::from([0.1,0.1,1.0,1.0]);
    let mut grid_opacity: u8 = 128;

    let mut show_demo = true;

    for port in serialport::available_ports().expect("No Ports found") {
        // 
        if port.port_name.contains("/dev/ttyUSB") | port.port_name.contains("/dev/ttyACM") {
            port_string = port.port_name.clone();
        }

        availible_ports.push(port.port_name);
    }
    let (waveform_tx,waveform_rx): (Sender<ScopeResponse>, Receiver<ScopeResponse>) = mpsc::channel();
    let (status_tx, status_rx): (Sender<ScopeStatus>, Receiver<ScopeStatus>) = mpsc::channel();

    let config_mutex: Arc<Mutex<CaptureConfig>> = Arc::new(Mutex::new(CaptureConfig {
        do_capture: false,
        open_port: false,
        two_stopbits: false,
        do_ri: true,
        do_s1: false,
        do_ro: false,
        port_name: port_string.clone(),
        channel: Channel::DISPLAY1,
        baudrate: 9600,
    }));

    let confix_mutex_guard: Arc<Mutex<CaptureConfig>> = Arc::clone(&config_mutex);

    
    // Data Capture thread
    thread::spawn(move || {

        let mut config: CaptureConfig;
        let mut stopbits: StopBits;
        'thread_loop: loop {
            // copy config from mutex
            if let Ok(ref mut mutex) = config_mutex_guard.try_lock() {
                config = (**mutex).clone();
            }
            else {
                // if we can't lock it, restart the loop, effectively spinlocking until we can get the config
                continue 'thread_loop;
            }
            
            // check if the port should be opened
            if config.open_port {
                // get the correct stop bit value
                if config.two_stopbits {stopbits = StopBits::Two;}
                else {stopbits = StopBits::One;}
                
                // initialize the capture
                let mut capture = OscilloscopeCapture::new(
                    &config.port_name.as_str(),
                    config.baudrate,
                    stopbits
                );

                // handle commands 
                if config.do_capture {
                    // create an empty response object
                    let mut response = ScopeResponse{
                        s1_result: false,
                        capture_conditions: String::new(),
                        waveform_data: Vec::<f64>::with_capacity(1000),
                        time_per_div: ValueUnitPair::default(),
                        voltage_per_div: ValueUnitPair::default()
                    };
                    
                    if config.do_s1 {
                        if let Err(e) = capture.send_s1() {
                            eprintln!("S1 Failed: {e}");
                            response.s1_result = false;
                            // Send S1 failure status message
                            status_tx.send(ScopeStatus::S1Fail).unwrap();
                            waveform_tx.send(response).unwrap();
                            continue 'thread_loop;
                        }
                        response.s1_result = true;
                        continue 'thread_loop;                        
                    }
                    else if config.do_ri {
                        status_tx.send(ScopeStatus::Ri).unwrap();

                        match capture.get_waveform_data(config.channel, 1.0) {
                            Ok(data) => {
                                response.voltage_per_div = data.2;
                                response.time_per_div = data.1;
                                response.waveform_data = data.0;

                                // send status message to main thread
                                status_tx.send(ScopeStatus::RiSuccess).unwrap();
                            },
                            // doing the error handling inside the capture thread allows us to use the status channel to display the current status more accurately
                            Err(e) => {
                                let message = match e {
                                    // at 9600 Baud every command fails a lot, so we will just retry until it succeeds, i guess
                                    OscilloscopeError::S1Failure => {ScopeStatus::S1Fail},
                                    OscilloscopeError::RiError => {ScopeStatus::RiFail},
                                    OscilloscopeError::RoError => {ScopeStatus::RoFail},
                                    OscilloscopeError::WriteError => {ScopeStatus::UnknownError},
                                };
                                sleep(Duration::from_millis(1000));
                                status_tx.send(message).unwrap();
                                continue 'thread_loop;
                            }
                        }
                        // send the response object back to the main frame through the waveform_tx channel
                        waveform_tx.send(response).unwrap();
                    }
                    else if config.do_ro {
                        println!("{:?}",capture.send_ro(config.channel));
                        // TODO? Idk this seems kinda useless
                        response.s1_result = true;
                        response.capture_conditions = String::from("TODO");
                        waveform_tx.send(response).unwrap()
                    }
                }
                //sleep(Duration::from_millis(500));


            }
            // if we don't have to do anything, take a nap
            else {
                sleep(Duration::from_millis(1000));
                status_tx.send(ScopeStatus::Idle).unwrap();
                continue 'thread_loop;
            }
        }
    });

    support::simple_init("scope-ui", move |_, ui| {
        let display_size = ui.io().display_size;
        let (mouse_x, _mouse_y) = ui.io().mouse_pos.into();

        //println!("{:?}", current_config.port_name);
        ui.window("Main Window, I guess?")
            .size(display_size, Condition::Always)
            .position([0.0,0.0], Condition::Always)
            .flags(WindowFlags::NO_DOCKING)
            .always_auto_resize(true)
            .bg_alpha(1.0)
            .movable(false)
            .collapsible(false)
            .bring_to_front_on_focus(false)
            .horizontal_scrollbar(false)
            .scrollable(false)
            .no_decoration()
            .build(|| {
                // empty window, for now
                ui.invisible_button("main_invis", [1.0,395.0]);
                if let Ok(a) = status_rx.try_recv() {
                    status_string = match a {
                         ScopeStatus::Idle => "Idle",
                         ScopeStatus::Ri => "Getting Waveform",
                         ScopeStatus::RiFail => "Failed to get Waveform",
                         ScopeStatus::RiSuccess => "Waveform captured",
                         ScopeStatus::S1 => "Testing Connection",
                         ScopeStatus::RoFail => "Failed to get Measurement Conditions",
                         ScopeStatus::S1Fail => "Connection Failed",
                         ScopeStatus::S1Success => "Connection Successful",
                         _ => "undefined"
                    };
                }
                ui.columns(5, "main_cols", false);
                ui.text(status_string);
                
            });
        
        ui.window("Interpolator Settings")
            .position([800.0,0.0], Condition::Always)
            .size([300.0,400.0], Condition::Always)
            .no_decoration()
            .movable(false)
            .collapsible(false)
            .resizable(false)
            .build(|| {
                ui.text("Interpolation Method");
                ui.columns(2, "interp_method", false);
                ui.radio_button("Linear", &mut interpolation_method, 0);
                ui.radio_button("Cosine", &mut interpolation_method, 1);
                ui.next_column();
                ui.radio_button("Catmull-Rom", &mut interpolation_method, 2);
                ui.radio_button("Bézier", &mut interpolation_method, 3);
                ui.radio_button("Bézier Variant", &mut interpolation_method, 4);
                ui.columns(1, "interp_samples", false);
                ui.separator();
                ui.slider("Samples", 1001, 16000, &mut interpol_samples);
                ui.slider("Step", 1, 20, &mut interpol_step);
                ui.separator();
                ui.slider("Samples 2", 1001, u16::MAX as usize, &mut interpol2_samples);
                ui.slider("Step 2", 1, 50, &mut interpol2_step);
                ui.separator();
                ui.slider("Window Size", 1, max_window_size, &mut avg_window_size);
            });
        
        ui.window("Draw Controls")
            .size([300.0,200.0], Condition::Always)
            .position([500.0,0.0], Condition::Always)
            .collapsible(false)
            .no_decoration()
            .resizable(false)
            .movable(false)
            .no_decoration()
            .build(|| {
                ui.columns(2, "Draw Control Columns", false);
                ui.set_column_width(0, 152.0);
                ui.checkbox("Draw Moving Average", &mut draw_average);
                ui.checkbox("Draw Trace", &mut draw_main_trace);
                ui.checkbox("Draw Grid", &mut draw_grid);
                ui.checkbox("Snap to trace", &mut snap_to_trace);
                ui.checkbox("Draw Dots", &mut draw_dots);
                ui.next_column();
                ui.text("Trace Thickness");
                ui.slider(" ", 1.0, 5.0, &mut trace_thickness);
                ui.text("Moving Average Thickness");
                ui.slider("   ", 1.0, 5.0, &mut avg_thickness);
                ui.text("Grid Opacity");
                ui.slider("    ", 1, 255, &mut grid_opacity);
                ui.columns(1, "Draw Control Columns 2", false);
                ui.separator();
                ui.slider("X Scale", 0.1, 10.0, &mut x_scale);
                ui.slider("Y Scale", 0.1, 10.0, &mut y_scale);
                if CollapsingHeader::new("Trace Colors")
                    .default_open(false)
                    .build(&ui) {
                        ui.columns(2, "Colors", true);
                        ui.color_picker4("Main Trace", &mut trace_color);
                        ui.next_column();
                        ui.color_picker4("Moving Average ", &mut avg_color);
                }
            }
        );
        

        ui.window("Drawing Window")
            .size([500.0,400.0], Condition::Appearing)
            .position([0.0,0.0], Condition::Always)
            .resizable(false)
            .collapsible(false)
            .no_decoration()
            .movable(false)
            .bg_alpha(1.0)
            .build(|| {
                // only allow moving the window when control is pressed
                let draw_list = ui.get_window_draw_list();
                let (window_width, window_height) = ui.window_size().into();
                let (win_x, win_y) = ui.window_pos().into();
                let interp_data_lin = linear_interpolate_samples(&waveform_buffer, interpol2_samples, time_per_div.value, interpol2_step);
                
                max_window_size = interp_data_lin.len() / 2;

                // increase samples by scrolling
                if ui.is_window_hovered() && ui.is_window_focused() && !ui.io().key_ctrl{
                    if ui.io().mouse_wheel < 0.0 && x_scale > 0.1 {
                        //interpol_samples -= 100;
                        x_scale -= 0.1;
                        
                    }
                    
                    else if ui.io().mouse_wheel > 0.0 {
                        //interpol_samples += 100;
                        x_scale += 0.1;
                    }
                }

                // 0 Linear
                // 1 Cosine
                // 2 Catmull-Rom
                // 3 Bézier
                // 4 Bézier variant 

                let interp_data:Vec<f64> = match interpolation_method {
                    0 => {linear_interpolate_samples(&interp_data_lin, interpol_samples, time_per_div.value, interpol_step)},
                    1 => {cosine_interpolate_samples(&interp_data_lin, interpol_samples, time_per_div.value, interpol_step)},
                    2 => {catmull_rom_interpolate_samples(&interp_data_lin, interpol_samples, time_per_div.value, interpol_step)},
                    3 => {bezier_interpolate_samples(&interp_data_lin, interpol_samples, time_per_div.value, interpol_step)},
                    4 => {bezier2_interpolate_samples(&interp_data_lin, interpol_samples, time_per_div.value, interpol_step)},
                    _ => {interp_data_lin.clone()}
                };

                let index_end = interp_data.len() - 1;

                // update the start index (mouse dragging moves waveform left and right)
                update_start_index(index_end, &mut index_start, &ui, x_scale);
    
                // draw background
                draw_list.add_rect(ui.window_pos(), [win_x + window_width, win_y + window_height], color::ImColor32::from_rgb(10, 10, 10)).filled(true).build();

                //draw grid lines
                if draw_grid {
                    let line_color = color::ImColor32::from_rgba(244, 244, 233, grid_opacity);
                    draw_grid_lines(line_color, 5.0, &ui, &draw_list);
                }

                // draw main trace
                if draw_main_trace {
                    if !draw_dots {
                        // draw lines at half opacity
                        draw_trace_lines(&interp_data, &draw_list, color::ImColor32::from_rgba_f32s(trace_color.x, trace_color.y, trace_color.z, trace_color.w / 2.0), index_start, index_end,(x_offset as f32, y_offset), (x_scale / 2.0, y_scale),trace_thickness ,&ui);
                        // draw dots over it
                        draw_trace_dots(&interp_data, &draw_list, color::ImColor32::from_rgba_f32s(trace_color.x, trace_color.y, trace_color.z, trace_color.w / 2.0), index_start, index_end,(x_offset as f32, y_offset), (x_scale / 2.0, y_scale),trace_thickness ,&ui);
                    }
                    else {
                        draw_trace_dots(&interp_data, &draw_list, color::ImColor32::from_rgba_f32s(trace_color.x, trace_color.y, trace_color.z, trace_color.w), index_start, index_end,(x_offset as f32, y_offset), (x_scale / 2.0, y_scale),trace_thickness ,&ui);
                    }
                    
                }
                
                // draw moving average trace
                if draw_average && avg_window_size < interp_data_lin.len() {
                    // calculate moving averages from the linearly interpolated trace
                    // x * time_per_div 
                    let mut moving_avg = processing::make_rectangle(voltage_per_div.value, voltage_per_div.value, time_per_div.value, 3.0);
                    
                    // sample down to main trace size
                    moving_avg = processing::linear_interpolate_samples(&moving_avg, interpol_samples, time_per_div.value, 1);
                    draw_trace_lines(&moving_avg, &draw_list, color::ImColor32::from_rgba_f32s(avg_color.x, avg_color.y, avg_color.z,avg_color.w), index_start, index_end, (x_offset as f32, y_offset), (x_scale / 2.0, y_scale),avg_thickness, &ui);
                }
                // draw things
                ui.text(format!("{}..{}", index_start, index_end));
                ui.text(format!("{}{}/div", voltage_per_div.value, voltage_per_div.unit_name));
                ui.text(format!("{}{}/div", time_per_div.value, time_per_div.unit_name));
                
                // only do this if the window is hovered, focused and the mouse position is valid (i.e the window is actively being used)
                if ui.is_window_hovered() && ui.is_current_mouse_pos_valid() && ui.is_window_focused() {
                    // change waveform scaling factor when ctrl + scroll
                    if ui.io().key_ctrl && ui.io().mouse_wheel < 0.0 && y_scale > 0.1 {y_scale += -0.1;}
                    else if ui.io().key_ctrl && ui.io().mouse_wheel > 0.0 && y_scale < 5.0 {y_scale += 0.1;}

                    x_offset = x_offset + ui.io().mouse_wheel as usize;
                    
                    // draw red dot cursor
                    let y_coord: f32;

                    // set circle y coordinate to trace when snep_to_trace = true
                    // TODO: make snap to trace less janky
                    if snap_to_trace {
                        let mut index = (((mouse_x - win_x).ceil() / (x_scale / 2.0)) + 1.0) as usize + index_start;
                        if index >= index_end {
                            index = index_end - 1;
                        }
                        ui.text(format!("{}", index));
                        y_coord = (((interp_data[index] as f32 * 255.0 * y_scale) + (win_y + 5.0) + (ui.window_size()[1]/2.0))) + y_offset;
                        draw_list.add_circle([mouse_x,y_coord], 2.0, color::ImColor32::from_rgb(255, 255, 255)).filled(true).build();
                        draw_list.add_text([mouse_x - 4.0, y_coord + 6.0], color::ImColor32::from_rgb(255, 255, 255), format!("Voltage: {:.3}{}", -(interp_data[index]), voltage_per_div.unit_name));
                    }
                }
            }
        );

        if let Ok(ref mut mutex) = config_mutex.lock()  {
            let mut current_config = (**mutex).clone();

            ui.window("Capture Controls")
                .size([300.0,200.0], Condition::Appearing)
                .position([500.0,200.0], Condition::Appearing)
                .collapsible(false)
                .movable(false)
                .resizable(false)
                .no_decoration()
                .build(|| {
                    let _tab = ui.tab_bar("capture_tabs");
                    if let Some(te) = ui.tab_item("Action") {
                        let disabled = ui.begin_disabled(current_config.open_port | do_capture);   
                        ui.radio_button("Test Connection", &mut mode_radiobutton, 0);
                        ui.radio_button("Get Conditions", &mut mode_radiobutton, 1);
                        ui.radio_button("Get Waveform", &mut mode_radiobutton, 2);

                        if ui.button_with_size("Capture Single", [150.0,25.0]) && !do_capture {
                            do_capture = true;
                            single_capture = true;
                        };
                        disabled.end();

                        ui.disabled(!do_capture, || {
                            do_capture = !ui.button_with_size("Stop", [150.0,25.0]) && do_capture;
                        });
                        // set all to false
                        current_config.do_ri = false;
                        current_config.do_ro = false;
                        current_config.do_s1 = false;
                        
                        match mode_radiobutton {
                            0 => {current_config.do_s1 = true;},
                            1 => {current_config.do_ro = true;},
                            2 => {current_config.do_ri = true;},
                            _ => {}
                        }
                        
                        current_config.do_capture = do_capture;
                        current_config.open_port = do_capture;
                        te.end();
                    };

                    if let Some(te) = ui.tab_item("Channel") {
                        let disabled = ui.begin_disabled(current_config.open_port); 
                        ui.radio_button("Display 1", &mut channel, Channel::DISPLAY1);
                        ui.radio_button("Display 2", &mut channel, Channel::DISPLAY2);
                        ui.radio_button("Save 1", &mut channel, Channel::SAVE1);
                        ui.radio_button("Save 2", &mut channel, Channel::SAVE2);

                        ui.next_column();
                        ui.text("Capture channel");

                        current_config.channel = channel;
                        disabled.end();

                        //let mut current_config = (**mutex).clone();

                        te.end();
                    }

                    if let Some(a) = ui.tab_item("Connection Settings") {
                    if CollapsingHeader::new("Baudrate Setup")
                    .default_open(false)
                    .build(&ui) {
                        let disabled = ui.begin_disabled(current_config.open_port);
                        // draw current config
                        ui.text(format!("Capture: {}", current_config.do_capture));
                        let mut baud_index = 5;

                        // get baudrate index
                        for (i, rate) in BAUDRATES.iter().enumerate() {
                            if *rate == current_config.baudrate.to_string().as_str() {
                                baud_index = i as i32;
                            }
                        }
                        ui.list_box("Baudrate", &mut baud_index, &BAUDRATES, 6);

                        if let Ok(a) = (BAUDRATES[baud_index as usize]).parse::<u32>() {
                            current_config.baudrate = a;
                        }
                        disabled.end()
                        
                    };

                if CollapsingHeader::new("Port Name")
                    .default_open(false)
                    .build(&ui) {
                        let disabled = ui.begin_disabled(current_config.open_port);
                        if let Some(_) = ui.begin_combo("Port", &port_string) {
                            for port in &availible_ports {
                                if &port_string == port {ui.set_item_default_focus();}
                                
                                let selected = ui.selectable_config(port)
                                    .selected(&port_string == port)
                                    .build();

                                if selected {port_string = port.clone();current_config.port_name = port.clone();} 
                            }
                        };
                        disabled.end();
                }
                
                ui.disabled(current_config.open_port, || {ui.checkbox("2 Stop Bits", &mut current_config.two_stopbits);});
                a.end();
            }
            });

            **mutex = current_config;
        };
        //ui.show_demo_window(&mut show_demo);

        // receive data from the data capture thread
        if let Ok(a) = waveform_rx.try_recv() {
            if a.waveform_data.len() > 0 {
                time_per_div = a.time_per_div;
                voltage_per_div = a.voltage_per_div;
                waveform_buffer = a.waveform_data;
            }
            
            // TODO continous capture
            if single_capture {
                do_capture = false;
            }
        }

        //ui.text(format!("{}", ui.io().framerate));
    });
}
