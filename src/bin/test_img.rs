use std::{fs, io::Write};

use derive_typst_intoval::{IntoDict, IntoValue};
use typst::{
    foundations::{Bytes, Dict, IntoValue},
    layout::PagedDocument,
};
use typst_as_lib::TypstEngine;

// main.rs
static TEMPLATE_FILE: &str = include_str!("./template.typ");
static FONT: &[u8] = include_bytes!("C:\\WINDOWS\\FONTS\\MSYHL.TTC");
static OUTPUT: &str = "./output.png";
//static IMAGE: &[u8] = include_bytes!("./templates/images/typst.png");

fn main() {
    // Read in fonts and the main source file.
    // We can use this template more than once, if needed (Possibly
    // with different input each time).
    let template = TypstEngine::builder()
        .main_file(TEMPLATE_FILE)
        .fonts([FONT])
        .build();

    // Run it
    let doc: PagedDocument = template
        .compile_with_input(dummy_data())
        .output
        .expect("typst::compile() returned an error!");

    let png = typst_render::render(doc.pages.first().unwrap(), 4.0)
        .encode_png()
        .expect("Failed to render PNk");
    fs::write(OUTPUT, &png).expect("Failed to write PNG data");

    // If you want to write the PDF instead, uncomment the following line:
    // fs::write(OUTPUT, doc.encode_pdf().expect("Failed to encode PDF")).expect("Failed to write output file");
}

// Some dummy content. We use `derive_typst_intoval` to easily
// create `Dict`s from structs by deriving `IntoDict`;
fn dummy_data() -> Content {
    Content {
        v: vec![
            ContentElement {
                heading: "Foo".to_owned(),
                text: Some("Hello World!".to_owned()),
                num1: 1,
                num2: Some(42),
                image: None, //Some(Bytes::new(IMAGE.to_vec())),
            },
            ContentElement {
                heading: "Bar".to_owned(),
                num1: 2,
                ..Default::default()
            },
        ],
    }
}

#[derive(Debug, Clone, IntoValue, IntoDict)]
struct Content {
    v: Vec<ContentElement>,
}

// Implement Into<Dict> manually, so we can just pass the struct
// to the compile function.
impl From<Content> for Dict {
    fn from(value: Content) -> Self {
        value.into_dict()
    }
}

#[derive(Debug, Clone, Default, IntoValue, IntoDict)]
struct ContentElement {
    heading: String,
    text: Option<String>,
    num1: i32,
    num2: Option<i32>,
    image: Option<Bytes>,
}
