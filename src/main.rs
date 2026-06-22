pub mod clushnr;

use std::f64::NAN;
use std::io::Read;
use std::ptr::with_exposed_provenance;
use reqwest::{Client, ClientBuilder};
use serde_json::Value;
use std::sync::Arc;
//use tokio::task::JoinSet;
use bounded_join_set::JoinSet;
use clap::{Arg, Command, ArgAction};

//Schema: https://www.dmtf.org/sites/default/files/standards/documents/DSP2046_2025.2.pdf

const CONCURRENCY:usize = 100;

struct Config {
    user: String,
    password: String,
    host: String,
    proxy: String,
}

struct SensorReading {
    reading: f64,
    reading_str: String,
    uppercaution: f64,
    uppercrit: f64,
    upperfatal: f64,
    lowercaution: f64,
    lowercrit: f64,
    lowerfatal: f64,
    url: String,
    host: String, //in case we're looking at multiple hosts concurrently
}

impl SensorReading {
    pub fn new() -> Self {
        Self {
            reading: f64::NAN,
            reading_str: "NaN".to_string(),
            uppercaution: f64::NAN,
            uppercrit: f64::NAN,
            upperfatal: f64::NAN,
            lowercaution: f64::NAN,
            lowercrit: f64::NAN,
            lowerfatal: f64::NAN,
            url: String::new(),
            host: String::new(),
        }
    }
}

fn parse_sensor_thresholds(json_blob:&serde_json::Value, reading:&mut SensorReading) {
    let mut threshold_obj = json_blob.as_object().unwrap().get("Thresholds");
    //there should be a Thresholds object but if we try to reuse this for Thermal or so on, maybe not
    //just handle both cases
    let threshold_obj = (threshold_obj).unwrap_or_else(|| return json_blob);
    for (k,v) in threshold_obj.as_object().unwrap() {
        match(k.as_str()) {
            //FIXME This is kind of clunky and will do something wrong if we get a number instead of a threshold object
            //Should never happen per the spec but I don't like it. On the other hand, much more compact code than it could be
            //unsure how to just break out of the match block if we get an invalid object
            "UpperCaution" => reading.uppercaution = v.get("Reading").unwrap_or_else(|| v).as_f64().unwrap_or_else(|| f64::NAN),
            "UpperCritical" => reading.uppercrit = v.get("Reading").unwrap_or_else(|| v).as_f64().unwrap_or_else(|| f64::NAN),
            "UpperFatal" => reading.upperfatal = v.get("Reading").unwrap_or_else(|| v).as_f64().unwrap_or_else(|| f64::NAN),
            "LowerCaution" => reading.lowercaution = v.get("Reading").unwrap_or_else(|| v).as_f64().unwrap_or_else(|| f64::NAN),
            "LowerCritical" => reading.lowercrit = v.get("Reading").unwrap_or_else(|| v).as_f64().unwrap_or_else(|| f64::NAN),
            "LowerFatal" => reading.lowerfatal = v.get("Reading").unwrap_or_else(|| v).as_f64().unwrap_or_else(|| f64::NAN),
            _ => (), //TODO there's a "UpperCautionUser" and so-on that can be configured by the user in redfish > 1.2
        }
    }
}

fn parse_sensor(json_blob:&serde_json::Value, url:String, host:String) -> SensorReading {
    let mut readingobj = SensorReading::new();
    readingobj.url = url;
    readingobj.host = host;
    if json_blob.as_object().unwrap().contains_key("Reading") {
        let val = json_blob.as_object().unwrap().get("Reading").unwrap().as_str();
        let mut reading = String::new();
        if val == None {
            reading = json_blob.as_object().unwrap().get("Reading").unwrap().as_number().unwrap().to_string();
        } else {
            reading = String::from(val.unwrap());
        }
        readingobj.reading = reading.parse::<f64>().unwrap_or_else(|_| f64::NAN);
        readingobj.reading_str = reading.to_string();
        parse_sensor_thresholds(&json_blob, &mut readingobj);
    }
    return readingobj;
}


async fn check_sensor(client:Arc<Client>, config:Arc<Config>, url: String) -> Result<(SensorReading), reqwest::Error> {
    //let client = ClientBuilder::new() //TODO make sure we're getting concurrency this way
    //    .danger_accept_invalid_certs(true)
    //    .danger_accept_invalid_hostnames(true)
    //    .build().unwrap();
    let host = config.host.clone();
    let rurl = format!("https://{}{}", &config.host, url);
    let res = client
        .get(rurl)
        .basic_auth(&config.user,Some(&config.password))
        .send()
        .await?;
    let raw_resp = res.text().await?;
    let json_root:Value = serde_json::from_str(raw_resp.as_str()).unwrap();
    let sr = parse_sensor(&json_root, url, host);
    //if !f64::is_nan(sr.reading) {
    //    println!("{}:{}", url, sr.reading);
    //}
   Ok((sr))
}
async fn get_sensors(client:Arc<Client>, config:Arc<Config> ) -> Result<(Vec<String>), reqwest::Error> {
    let mut sensor_urls:Vec<String> = Vec::new();
    let url = format!("https://{}/redfish/v1/Chassis/Enclosure/Sensors", &config.host);

    let res = client
        .get(url)
        .basic_auth(&config.user,Some(&config.password))
        .send()
        .await?;
    let resp_json = res.text().await?;
    let json:Value = serde_json::from_str(&*resp_json).unwrap(); //todo check this
    if !json.as_object().unwrap().contains_key("Members") {
        println!("A bad thing happened, here's the json: {}", json);
    }
    for k in json.as_object().unwrap().get("Members").unwrap().as_array().unwrap() {
        let surl = String::from(k.as_object().unwrap().get("@odata.id").unwrap().as_str().unwrap());
        sensor_urls.push(surl);
    }
    Ok((sensor_urls))
}

fn print_sensor_caution_human(url:String, reading:f64, threshold:f64, sense:String, host:String) {
    println!("{}: {} is {} than threshold. Measurement: {} Threshold: {}", host, url, sense, reading, threshold);
}

async fn print_human_healthcheck(client:Arc<Client>, config:Arc<Config> ) -> Result<(), reqwest::Error> {
    let sensor_urls = get_sensors(client.clone(), config.clone()).await?;
    let mut js = JoinSet::new(CONCURRENCY);
    for s in sensor_urls { //concurrent queries seem to save ~50% of the query time on iLO
        let sensordata = js.spawn(check_sensor(Arc::clone(&client), Arc::clone(&config), s.clone()));
    }
    while let Some(result) = js.join_next().await {
        let sensordata = result.unwrap().unwrap(); //TODO should probably implement the error case
        if sensordata.reading_str == "NaN" || f64::is_nan(sensordata.reading) {
            //probably a sensor without data/not implemented/etc
            continue;
        }
        || -> () { //I really just want goto so that we don't print every "fatal" three times but this will work
            if sensordata.reading < sensordata.lowerfatal {
                print_sensor_caution_human(sensordata.url.clone(), sensordata.reading, sensordata.lowerfatal, String::from("less"), sensordata.host.clone());
                return;
            }
            if sensordata.reading < sensordata.lowercrit {
                print_sensor_caution_human(sensordata.url.clone(), sensordata.reading, sensordata.lowercrit, String::from("less"), sensordata.host.clone());
                return
            }
            //We *should* be able to nest these but do we really trust hardware vendors?
            if sensordata.reading < sensordata.lowercaution {
                print_sensor_caution_human(sensordata.url.clone(), sensordata.reading, sensordata.lowercaution, String::from("less"), sensordata.host.clone());
                return;
            }
        }();

        || -> () {
            if sensordata.reading > sensordata.upperfatal {
                print_sensor_caution_human(sensordata.url.clone(), sensordata.reading, sensordata.upperfatal, String::from("greater"), sensordata.host.clone());
                return;
            }
            if sensordata.reading > sensordata.uppercrit {
                print_sensor_caution_human(sensordata.url.clone(), sensordata.reading, sensordata.uppercrit, String::from("greater"), sensordata.host.clone());
                return
            }
            //We *should* be able to nest these but do we really trust hardware vendors?
            if sensordata.reading > sensordata.uppercaution {
                print_sensor_caution_human(sensordata.url.clone(), sensordata.reading, sensordata.uppercaution, String::from("greater"), sensordata.host.clone());
                return;
            }
        }();

    } //while join
    Ok(())
}

async fn print_human_sensorlist(client:Arc<Client>, config:Arc<Config> ) -> Result<(), reqwest::Error> {
    let sensor_urls = get_sensors(client.clone(), config.clone()).await?;
    let mut js = JoinSet::new(CONCURRENCY);
    for s in sensor_urls { //concurrent queries seem to save ~50% of the query time on iLO
        let sensordata = js.spawn(check_sensor(Arc::clone(&client), Arc::clone(&config), s.clone()));
    }
    while let Some(result) = js.join_next().await {
        let sensordata = result.unwrap().unwrap(); //TODO should probably implement the error case
        if f64::is_nan(sensordata.reading) {
            if sensordata.reading_str == "NaN" {
                (); //A sensor for which there's a url in the list but no data
                    //HPE's implementation returns "Not Found" in this case rather than actual json
                    //FIXME, should probably add a "is_valid" flag instead of comparing strings
            } else {
                println!("{}: {}: {}", sensordata.host, sensordata.url, sensordata.reading_str);
            }
        } else {
            println!("{}: {}: {}", sensordata.host, sensordata.url, sensordata.reading);
        }
    }
    Ok(())
}

//old test function, not used
async fn get_sensors_test(client:Arc<Client>, config:Arc<Config> ) -> Result<(), reqwest::Error> {
    let url = format!("https://{}/redfish/v1/Chassis/Enclosure/Sensors", &config.host);

    let res = client
        .get(url)
        .basic_auth(&config.user,Some(&config.password))
        .send()
        .await?;
    let resp_json = res.text().await?;
    let json:Value = serde_json::from_str(&*resp_json).unwrap(); //todo check this
    let mut js = JoinSet::new(CONCURRENCY);
    if !json.as_object().unwrap().contains_key("Members") {
        println!("A bad thing happened, here's the json: {}", json);
    }
    for k in json.as_object().unwrap().get("Members").unwrap().as_array().unwrap() {
        let surl = String::from(k.as_object().unwrap().get("@odata.id").unwrap().as_str().unwrap());
        js.spawn(check_sensor(Arc::clone(&client), Arc::clone(&config), surl));
    }
    js.join_all().await;
    for (k, v) in json.as_object().unwrap() {
        //println!("{}: {}", k, v);
    }
    //println!("{:#?}", resp_json);
    Ok(())
}

fn parse_noderange(nr:String) -> Vec<String> {
    let mut hosts:Vec<String> = Vec::new();
    //TODO
    hosts.push(nr);
    return(hosts);
}
#[tokio::main]
async fn main() -> Result<(), reqwest::Error> {
    let mut user=String::from("root");
    let mut password=String::from("toor");
    let mut proxy = String::new();
    let mut hostlist:Vec<String> = Vec::new();

    let args = clap::command!()
        .subcommand_required(true)
        .subcommand(
            Command::new("healthcheck")
                .about("Check Sensor Thresholds")
                .arg(clap::arg!([COMMAND]))
        )
        .subcommand(
            Command::new("sensors")
                .about("Print Sensor Values")
                .arg(clap::arg!([COMMAND]))
        )
        .arg(clap::arg!(
            -r --proxy <proxy>)
            .required(false))
        .arg(clap::arg!(
            -u --user <user>)
            .required(false))
        .arg(clap::arg!(
            -p --password <password>)
            .required(false))
        .arg(clap::arg!(
            -n --noderange <noderange>)
            .required(false))
        .arg(Arg::new("clush-noderange")
            .long("clush-noderange")
            .help("Specify a clush nodegroup")
            .action(ArgAction::Set)
            .short('N'))
        .get_matches();
    if let Some(p) = args.get_one::<String>("proxy") {
        proxy = String::from(p);
    }
    if let Some(p) = args.get_one::<String>("password") {
        password = String::from(p);
    }
    if let Some(u) = args.get_one::<String>("user") {
        user = String::from(u);
    }
    if let Some(nr) = args.get_one::<String>("noderange") {
        hostlist = parse_noderange(String::from(nr));
    }
    if let Some(nr) = args.get_one::<String>("clush-noderange") {
        hostlist = clushnr::clushnr::get_nodes(String::from(nr));
    }

    let default = String::from("https://localhost:8088");
    let mut js = JoinSet::new(CONCURRENCY);
    for host in hostlist {
        let c = Arc::new(Config {
            user: user.clone(),
            password: password.clone(),
            host: host.clone(),
            proxy: proxy.clone(),
        });
        let mut cb = ClientBuilder::new()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true);
        if !c.proxy.is_empty() {
            cb = cb.proxy(reqwest::Proxy::https(c.proxy.as_str())?);
        }
        let client = cb.build().unwrap();
        let clientptr = Arc::new(client);
        match args.subcommand() {
            Some(("healthcheck", _)) => {js.spawn(print_human_healthcheck(Arc::clone(&clientptr), Arc::clone(&c)));}
            Some(("sensors", _)) => {js.spawn(print_human_sensorlist(Arc::clone(&clientptr), Arc::clone(&c)));}
            _ => unreachable!("Invalid Operation Specified"),
        }

        //print_human_sensorlist(Arc::clone(&clientptr), Arc::clone(&c)).await?;
    }
    js.join_all().await;
    Ok(())
}
