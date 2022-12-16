//! Interoperability test program for `RustDDS` library

#![deny(clippy::all)]
#![warn(clippy::pedantic)]

use std::{
  io,
  time::{Duration, Instant},
};

use log::{debug, error, trace, LevelFilter};
use log4rs::{
  append::console::ConsoleAppender,
  config::{Appender, Root},
  Config,
};
use rustdds::{
  DomainParticipant, Keyed, QosPolicyBuilder, StatusEvented, TopicDescription, TopicKind,
};
use rustdds::policy::{Deadline, Durability, History, Reliability}; /* import all QoS
                                                                     * policies directly */

use rustdds::with_key::DataReaderStream;

use serde::{Deserialize, Serialize};
use clap::{Arg, ArgMatches, Command}; // command line argument processing
use mio_06::{Events, Poll, PollOpt, Ready, Token}; // polling
use mio_extras::channel; // pollable channel
use rand::prelude::*;

use smol::{prelude::*, Async};
use futures::FutureExt;

#[derive(Serialize, Deserialize, Clone)]
struct Shape {
  color: String,
  x: i32,
  y: i32,
  shapesize: i32,
}

impl Keyed for Shape {
  type K = String;
  fn key(&self) -> String {
    self.color.clone()
  }
}

const DA_WIDTH: i32 = 240;
const DA_HEIGHT: i32 = 270;

const STOP_PROGRAM: Token = Token(0);
const READER_READY: Token = Token(1);
const READER_STATUS_READY: Token = Token(2);
const WRITER_STATUS_READY: Token = Token(3);

#[allow(clippy::too_many_lines)]
fn main() -> std::io::Result<()> {
  configure_logging();
  let matches = get_matches();

  // Process command line arguments
  let topic_name = matches.value_of("topic").unwrap_or("Square");
  let domain_id = matches
    .value_of("domain_id")
    .unwrap_or("0")
    .parse::<u16>()
    .unwrap_or(0);
  let color = matches.value_of("color").unwrap_or("BLUE");

  let domain_participant = DomainParticipant::new(domain_id)
    .unwrap_or_else(|e| panic!("DomainParticipant construction failed: {:?}", e));

  let mut qos_b = QosPolicyBuilder::new()
    .reliability(if matches.is_present("reliable") {
      Reliability::Reliable {
        max_blocking_time: rustdds::Duration::DURATION_ZERO,
      }
    } else {
      Reliability::BestEffort
    })
    .durability(match matches.value_of("durability") {
      Some("l") => Durability::TransientLocal,
      Some("t") => Durability::Transient,
      Some("p") => Durability::Persistent,
      _ => Durability::Volatile,
    })
    .history(match matches.value_of("history_depth").map(str::parse) {
      None | Some(Err(_)) => History::KeepAll,
      Some(Ok(d)) => {
        if d < 0 {
          History::KeepAll
        } else {
          History::KeepLast { depth: d }
        }
      }
    });
  let deadline_policy = match matches.value_of("deadline") {
    None => None,
    Some(dl) => match dl.parse::<f64>() {
      Ok(d) => Some(Deadline(rustdds::Duration::from_frac_seconds(d))),
      Err(e) => panic!("Expected numeric value for deadline. {:?}", e),
    },
  };

  if let Some(dl) = deadline_policy {
    qos_b = qos_b.deadline(dl);
  }

  assert!(
    !matches.is_present("partition"),
    "QoS policy Partition is not yet implemented."
  );

  assert!(
    !matches.is_present("interval"),
    "QoS policy Time Based Filter is not yet implemented."
  );

  assert!(
    !matches.is_present("ownership_strength"),
    "QoS policy Ownership Strength is not yet implemented."
  );

  let qos = qos_b.build();

  let loop_delay: Duration = match deadline_policy {
    None => Duration::from_millis(200), // This is the default rate
    Some(Deadline(dd)) => Duration::from(dd).mul_f32(0.8), // slightly faster than dealine
  };

  let topic = domain_participant
    .create_topic(
      topic_name.to_string(),
      "ShapeType".to_string(),
      &qos,
      TopicKind::WithKey,
    )
    .unwrap_or_else(|e| panic!("create_topic failed: {:?}", e));
  println!(
    "Topic name is {}. Type is {}.",
    topic.name(),
    topic.get_type().name()
  );

  // Set Ctrl-C handler
  let (stop_sender, stop_receiver) = smol::channel::bounded(1);
  ctrlc::set_handler(move || {
    smol::block_on( async {
      stop_sender.send(()).await.unwrap_or(());
      // ignore errors, as we are quitting anyway
    })
  })
  .expect("Error setting Ctrl-C handler");
  println!("Press Ctrl-C to quit.");


  let subscriber = domain_participant.create_subscriber(&qos).unwrap();
  let mut reader = subscriber
      .create_datareader_cdr::<Shape>(&topic, Some(qos))
      .unwrap();

  let mut datareader_stream = DataReaderStream::new(reader);

  smol::block_on(async {
    println!("async started");
    let mut run = true;
    let mut stop = stop_receiver.recv().fuse();
    let mut ds = datareader_stream.next().fuse();
    while run {
      futures::select! {
        _ = stop => run = false,
        r = ds => {
          match r {
            Some(Ok(v)) =>
              match v {
                  Ok(sample) => println!(
                      "{:10.10} {:10.10} {:3.3} {:3.3} [{}]",
                      topic.name(),
                      sample.color,
                      sample.x,
                      sample.y,
                      sample.shapesize,
                    ),
                  Err(key) => println!("Disposed key {:?}", key),
              }
            Some(Err(e)) => {
              error!("{:?}",e);
              break;
            }
            None => run = false,
          }
        }      
      }
    }

    Ok(())
  })

}

fn configure_logging() {
  // initialize logging, preferably from config file
  log4rs::init_file(
    "logging-config.yaml",
    log4rs::config::Deserializers::default(),
  )
  .unwrap_or_else(|e| {
    match e.downcast_ref::<io::Error>() {
      // Config file did not work. If it is a simple "No such file or directory", then
      // substitute some default config.
      Some(os_err) if os_err.kind() == io::ErrorKind::NotFound => {
        println!("No config file found in current working directory.");
        let stdout = ConsoleAppender::builder().build();
        let conf = Config::builder()
          .appender(Appender::builder().build("stdout", Box::new(stdout)))
          .build(Root::builder().appender("stdout").build(LevelFilter::Error))
          .unwrap();
        log4rs::init_config(conf).unwrap();
      }
      // Give up.
      other_error => panic!("Config problem: {:?}", other_error),
    }
  });
}

fn get_matches() -> ArgMatches {
  Command::new("RustDDS-interop")
    .version("0.2.2")
    .author("Juhana Helovuo <juhe@iki.fi>")
    .about("Command-line \"shapes\" interoperability test.")
    .arg(
      Arg::new("domain_id")
        .short('d')
        .value_name("id")
        .help("Sets the DDS domain id number")
        .takes_value(true),
    )
    .arg(
      Arg::new("topic")
        .short('t')
        .value_name("name")
        .help("Sets the topic name")
        .takes_value(true)
        .required(true),
    )
    .arg(
      Arg::new("color")
        .short('c')
        .value_name("color")
        .help("Color to publish (or filter)")
        .takes_value(true),
    )
    .arg(
      Arg::new("durability")
        .short('D')
        .value_name("durability")
        .help("Set durability")
        .takes_value(true)
        .possible_values(&["v", "l", "t", "p"]),
    )
    .arg(
      Arg::new("publisher")
        .help("Act as publisher")
        .short('P')
        .required_unless_present("subscriber"),
    )
    .arg(
      Arg::new("subscriber")
        .help("Act as subscriber")
        .short('S')
        .required_unless_present("publisher"),
    )
    .arg(
      Arg::new("best_effort")
        .help("BEST_EFFORT reliability")
        .short('b')
        .conflicts_with("reliable"),
    )
    .arg(
      Arg::new("reliable")
        .help("RELIABLE reliability")
        .short('r')
        .conflicts_with("best_effort"),
    )
    .arg(
      Arg::new("history_depth")
        .help("Keep history depth")
        .short('k')
        .takes_value(true)
        .value_name("depth"),
    )
    .arg(
      Arg::new("deadline")
        .help("Set a 'deadline' with interval (seconds)")
        .short('f')
        .takes_value(true)
        .value_name("interval"),
    )
    .arg(
      Arg::new("partition")
        .help("Set a 'partition' string")
        .short('p')
        .takes_value(true)
        .value_name("partition"),
    )
    .arg(
      Arg::new("interval")
        .help("Apply 'time based filter' with interval (seconds)")
        .short('i')
        .takes_value(true)
        .value_name("interval"),
    )
    .arg(
      Arg::new("ownership_strength")
        .help("Set ownership strength [-1: SHARED]")
        .short('s')
        .takes_value(true)
        .value_name("strength"),
    )
    .get_matches()
}

#[allow(clippy::similar_names)]
fn move_shape(shape: Shape, xv: i32, yv: i32) -> (Shape, i32, i32) {
  let half_size = shape.shapesize / 2 + 1;
  let mut x = shape.x + xv;
  let mut y = shape.y + yv;

  let mut xv_new = xv;
  let mut yv_new = yv;

  if x < half_size {
    x = half_size;
    xv_new = -xv;
  }
  if x > DA_WIDTH - half_size {
    x = DA_WIDTH - half_size;
    xv_new = -xv;
  }
  if y < half_size {
    y = half_size;
    yv_new = -yv;
  }
  if y > DA_HEIGHT - half_size {
    y = DA_HEIGHT - half_size;
    yv_new = -yv;
  }
  (
    Shape {
      color: shape.color,
      x,
      y,
      shapesize: shape.shapesize,
    },
    xv_new,
    yv_new,
  )
}