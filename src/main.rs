use hyper::{
    http::Uri,
    body::{Body, Bytes, HttpBody, to_bytes},
    Client
};
use hyper_tls::HttpsConnector;
use std::{error::Error, str::from_utf8, io::{self, Write}};
use scraper::{Html, Selector, Node, ElementRef, element_ref::Text};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let https = HttpsConnector::new();
    let client = Client::builder().build::<_, hyper::Body>(https);
    let uri: Uri = "https://www.pro-football-reference.com/boxscores/202111280nyg.htm".parse()?;

    let mut resp = client.get(uri).await?;
    // println!("Resp status: {}", resp.status());

    let mut body = resp.into_body();
    // NOTE response header says it is UTF-8 but UTF-8 parsing fails...
    // `file` claims the bytes are actually ISO-8859-1.
    // TODO find a valid way to convert to text
    let bytes = to_bytes(body).await.unwrap();
    let s: String = bytes.iter().map(|&c| c as char).collect();

    let html_doc = Html::parse_document(&s);

    let offense_selector = Selector::parse("#player_offense").unwrap();
    let tbody_selector= Selector::parse("tbody").unwrap();

    let offense_table = html_doc.select(&offense_selector).next().unwrap();
    let table_body = offense_table.select(&tbody_selector).next().unwrap();

    println!("{}", table_body.html());
    'rowlabel: for data_row in table_body.children() {

        for data_elt in data_row.children() {
            let stat_name_opt = match data_elt.value() {
                Node::Element(elt) => elt.attr("data-stat"),
                _ => None
            };

            let stat_name = match stat_name_opt {
                Some(s) => s,
                None => {
                    // This indicates the table body row is not a data row.
                    continue 'rowlabel
                }
            };

            let data_val_elt = if stat_name == "player" {
                data_elt
                .first_child()
                .and_then(|n| n.first_child())
                .map(|n| n.value())
            } else {
                data_elt
                .first_child()
                .map(|v| v.value())
            };

            let data_val: Option<String> = match data_val_elt {
                // TODO get rid of allocation via format here
                Some(Node::Text(t)) => Some(format!("{}", t.text)),
                _ => None
            };

            println!("stat {}, val {:?}", stat_name, data_val);
        }
    }

    Ok(())
}