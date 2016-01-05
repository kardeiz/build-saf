extern crate getopts;
extern crate encoding;
extern crate csv;
extern crate handlebars;
extern crate rustc_serialize;
extern crate glob;
extern crate zip;
#[macro_use] extern crate maplit;

use getopts::Options;

use handlebars::Handlebars;

use rustc_serialize::json::{Json, ToJson};

use encoding::label::encoding_from_whatwg_label;

pub use std::io::prelude::*;
pub use std::env;
pub use std::fs;
pub use std::collections::{HashMap, BTreeMap};

const OUTPUT_DIR: &'static str = "SimpleArchiveFormat";

fn print_usage(program: &str, opts: Options) {
  let brief = format!("Usage: {} FILE [options]", program);
  print!("{}", opts.usage(&brief));
}

pub struct DcEntry {
  pub element: String,
  pub qualifier: String,
  pub values: Vec<String>
}

impl DcEntry {

  fn blank(&self) -> bool {
    self.values.is_empty() || 
      self.values.iter().all(|x| x.is_empty() )
  }

  fn real_values(&self) -> Vec<String> {
    self.values.clone().into_iter().filter(|x| !x.is_empty() ).collect()
  }


}

impl ToJson for DcEntry {
  fn to_json(&self) -> Json {
    (btreemap!{
      "element".into() => self.element.to_json(),
      "qualifier".into() => self.qualifier.to_json(),
      "values".into() => self.real_values().to_json()
    }).to_json()
  }
}

pub mod utils {
  use super::*;
  use std::path::PathBuf;
  use encoding; 
  use encoding::types::EncodingRef;

  pub fn read_file(path: &PathBuf, enc: EncodingRef) -> Result<String, String> {
    let mut file = try!(fs::File::open(path).map_err(|e| format!("{}", e) ));
    let mut content = Vec::new();
    try!(file.read_to_end(&mut content).map_err(|e| format!("{}", e) ));
    enc.decode(&content, encoding::DecoderTrap::Replace).map_err(|e| e.into_owned() )
  }

  pub fn get_filename_tuple<'a>(map: &'a HashMap<String, &'a String>) 
    -> (&'a String, &'a &'a String) {
    let alts = (
      || map.iter().filter(|&(k, v)| k.starts_with("filename")).next(),
      || map.iter().filter(|&(k, v)| k.starts_with("file name")).next(),
      || map.iter().filter(|&(k, v)| k.starts_with("bitstream")).next()
    );
    alts.0().or_else(alts.1).or_else(alts.2).expect("No filename column")
  }

}


fn main() {

  let mut handlebars = Handlebars::new();

  handlebars.register_template_string("dc", 
    include_str!("../templates/dublin_core.hbs").to_string());


  let current_dir = env::current_dir().unwrap();

  let args: Vec<String> = env::args().collect();
  
  let program = args[0].clone();

  let mut opts = Options::new();
  
  opts.optopt("c", "csv", "Filename with path of the CSV spreadsheet", "FILE");
  opts.optflag("h", "help", "Print this help menu");
  opts.optflag("e", "encoding", "Encoding of CSV file");
  opts.optflag("z", "zip", "Zip the output");

  let matches = match opts.parse(&args[1..]) {
    Ok(m) => { m }
    Err(f) => { panic!(f.to_string()) }
  };
  
  if matches.opt_present("h") {
    print_usage(&program, opts);
    return;
  }

  let encoding = matches.opt_str("e")
    .and_then(|e| encoding_from_whatwg_label(&e) )
    .unwrap_or_else(|| encoding_from_whatwg_label("windows-1252").unwrap() );

  let csv_path = {
    let name = matches.opt_str("c").expect("No csv specified");
    current_dir.join(name)
  };

  let base_dir = &csv_path.parent().map(|p| p.to_path_buf() ).unwrap();

  let mut csv = {
    let file = utils::read_file(&csv_path, encoding).unwrap();
    csv::Reader::from_string(file)
  };

  let headers = csv.headers().expect("Error with CSV");

  for (i, row) in csv.records().enumerate() {
    let row = row.unwrap();

    let folder_name = format!("item_{:0>5}", &i);
    let folder_path = &base_dir.join(OUTPUT_DIR).join(&folder_name);

    std::fs::create_dir_all(&folder_path).expect("Making dir failed");

    let zipped: HashMap<_, _> = 
      headers.clone().into_iter().zip(row.iter()).collect();

    let mut entries = Vec::new();

    for (k, v) in zipped.iter().filter(|&(k, v)| k.starts_with("dc.") ) {
      let x: Vec<&str> = k.split(".").collect();
      let element   = x.get(1).expect("CSV error").to_string();
      let qualifier = x.get(2).map(|x| x.to_string() ).unwrap_or_else(|| "none".to_string() );

      let values: Vec<String> = v.split("||").map(|x| x.to_string() ).collect();

      let entry = DcEntry {
        element: element,
        qualifier: qualifier,
        values: values
      };

      if !entry.blank() {
        entries.push(entry);
      }

    }

    let mut contents = 
      fs::File::create(&folder_path.join("contents")).unwrap();

    let (file_name_key, file_name_vals) = utils::get_filename_tuple(&zipped);

    let mut file_name_opts: Vec<&str> = file_name_key.split("__").collect();

    file_name_opts.remove(0);

    let file_name_exts: Vec<&str> = file_name_vals.split("||").collect();

    for file_name_ext in file_name_exts.iter() {
      let mut ext: Vec<&str> = file_name_ext.split("__").collect();

      ext.append(&mut file_name_opts.clone());

      let file_name = ext.get(0).unwrap();
      let file_all  = ext.join("\t");

      fs::copy(&base_dir.join(&file_name), 
        &folder_path.join(&file_name)).unwrap();
      
      contents.write_all(&file_all.as_bytes()).unwrap();
      contents.write_all("\r\n".as_bytes()).unwrap();

    }

    let result = handlebars.render("dc", &entries);

    let mut dublin_core_xml = 
      fs::File::create(&folder_path.join("dublin_core.xml")).unwrap();
    
    dublin_core_xml.write_all(result.unwrap().as_bytes());

    if matches.opt_present("z") {

      let mut zip_file = fs::File::create(
        &base_dir.join([OUTPUT_DIR, ".zip"].concat())).unwrap();

      let mut zip = zip::ZipWriter::new(zip_file);

      env::set_current_dir(&base_dir.join(OUTPUT_DIR)).unwrap();

      for path in glob::glob("**/*").unwrap()
        .filter_map(Result::ok)
        .filter(|x| fs::metadata(x).map(|x| x.is_file() ).unwrap_or(false) ) {
        
        let mut bytes = Vec::new();

        if let Ok(mut f) = fs::File::open(&path) {
          if let Ok(_) = f.read_to_end(&mut bytes) {
            if let Some(name) = path.to_str() {
              zip.start_file(name, zip::CompressionMethod::Stored).unwrap();
              zip.write_all(&bytes[..]).unwrap();
            }
          }
        }
      }
    }
  }

}
