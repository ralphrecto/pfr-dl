use futures::future::join_all;
use hyper::{
    http::Uri,
    body::{to_bytes},
    Client, client::HttpConnector
};
use hyper_tls::HttpsConnector;
use std::{error::Error, collections::HashMap};
use scraper::{Html, Selector, Node, ElementRef};
use regex::{Regex, Captures};
use lazy_static::lazy_static;

lazy_static!{
    static ref GAME_ID_REGEX: Regex = Regex::new(r".*/(\w+)\.htm").unwrap();
    static ref WEEK_NUM_REGEX: Regex = Regex::new(r".*/(\d{4})/week_(\d)+\.htm").unwrap();
}
const PFR_DOMAIN: &'static str = "https://www.pro-football-reference.com";

#[derive(Debug)]
struct PlayerGameStats<'a> {
    player_id: &'a str,
    player_name: &'a str,
    stats: HashMap<&'a str, &'a str>
}

#[derive(Debug)]
struct GameStats<'a> {
    game_id: &'a str,
    player_stats: Vec<PlayerGameStats<'a>>
}

#[derive(Debug)]
struct GameInfo<'a> {
    year: u32,
    week_num: u32,
    stats: GameStats<'a>
}

fn selector(selector_str: &str) -> Selector {
    Selector::parse(selector_str).unwrap()
}

fn parse_a_href(elt_ref: ElementRef) -> Uri {
    elt_ref.value()
        .attr("href")
        .and_then(|link| {
            if link.starts_with("/") {
                format!("{}{}", PFR_DOMAIN, link).parse().ok()
            } else {
                link.parse().ok()
            }
        }).unwrap()
}

fn parse_player_stats_table<'a>(uri: &Uri, html_doc: &'a Html, table_selector_str: &str) -> Result<Vec<PlayerGameStats<'a>>, String> {
    let table_selector = selector(table_selector_str);
    let table_elt = match html_doc.select(&table_selector).next() {
        Some(elt) => elt,
        None => return Err(format!("Game log at {}: no table element for selector '{}' found", uri, table_selector_str))
    };

    let tbody_selector= selector("tbody");
    let table_body = table_elt.select(&tbody_selector).next().unwrap();

    let mut retlist: Vec<PlayerGameStats> = vec![];
    'rowlabel: for table_row in table_body.children() {

        let mut player_data: HashMap<&str, &str> = HashMap::new();

        for table_data_noderef in table_row.children() {
            let table_data_elt = match table_data_noderef.value() {
                Node::Element(elt) => elt,
                _ => continue
            };

            // Handle player id.

            match table_data_elt.attr("data-append-csv") {
                Some(id) => {
                    player_data.insert("player_id", id);
                },
                None => ()
            };

            // Handle stat.

            let stat_name = match table_data_elt.attr("data-stat") {
                Some(s) => s,
                None => {
                    // This indicates the table body row is not a data row.
                    continue 'rowlabel
                }
            };

            let data_val_elt = if stat_name == "player" {
                table_data_noderef
                    .first_child()
                    .and_then(|n| n.first_child())
                    .map(|n| n.value())
            } else {
                table_data_noderef
                    .first_child()
                    .map(|v| v.value())
            };

            match data_val_elt {
                Some(Node::Text(t)) => {
                    player_data.insert(stat_name, t.text.as_ref());
                },
                _ => ()
            };
        }

        match (player_data.get("player_id"), player_data.get("player")) {
            (Some(&player_id), Some(&player_name)) => {
                retlist.push(PlayerGameStats {
                    player_id,
                    player_name,
                    stats: player_data
                });
            },
            _ => ()
        }
    }

    Ok(retlist)
}

fn parse_game_log<'a>(game_log_uri: &Uri, game_log_html: &'a Html) -> Result<GameStats<'a>, String> {
    let mut player_stats: Vec<PlayerGameStats> = vec![];

    let game_link_selector = selector("link[rel=canonical]");
    let game_link = game_log_html.select(&game_link_selector).next().unwrap().value().attr("href").unwrap();
    let game_id = GAME_ID_REGEX.captures(game_link).unwrap().get(1).unwrap().as_str();

    // Mandatory stats.
    player_stats.extend(parse_player_stats_table(game_log_uri, game_log_html, "#player_offense")?);
    player_stats.extend(parse_player_stats_table(game_log_uri, game_log_html, "#player_defense")?);
    player_stats.extend(parse_player_stats_table(game_log_uri, game_log_html, "#returns")?);
    player_stats.extend(parse_player_stats_table(game_log_uri, game_log_html, "#kicking")?);

    // Optional advanced stats.
    let adv_passing = parse_player_stats_table(game_log_uri, game_log_html, "#passing_advanced");
    let adv_rushing = parse_player_stats_table(game_log_uri, game_log_html, "#rushing_advanced");
    let adv_receiving = parse_player_stats_table(game_log_uri, game_log_html, "#receiving_advanced");
    let adv_def = parse_player_stats_table(game_log_uri, game_log_html, "#defense_advanced");

    for adv_stats in [adv_passing, adv_rushing, adv_receiving, adv_def] {
        match adv_stats {
            Ok(stats) => {
                player_stats.extend(stats);
            }
            Err(_) => ()
        }
    }

    Ok(GameStats { game_id, player_stats })
}

fn parse_season_week_page<'a>(season_week_log_html: &'a Html) -> Vec<Uri> {
    season_week_log_html
        .select(&selector(".gamelink a"))
        .map(parse_a_href)
        .collect()
}

fn parse_season_page<'a>(season_week_log_html: &'a Html) -> Vec<Uri> {
    season_week_log_html
        .select(&selector("#div_week_games a"))
        .filter(|elt_ref| {
            let text_opt = elt_ref
                .first_child()
                .map(|noderef| match noderef.value() {
                    Node::Text(t) => t.starts_with("Week"),
                    _ => false
                });

            text_opt.unwrap_or(false)
        }).map(parse_a_href)
        .collect()
}


async fn fetch_uri(client: &Client<HttpsConnector<HttpConnector>>, uri: Uri) -> Result<String, Box<dyn Error + Send + Sync>>  {
    let mut resp = client.get(uri).await?;
    // println!("Resp status: {}", resp.status());

    let mut body = resp.into_body();
    // NOTE response header says it is UTF-8 but UTF-8 parsing fails...
    // `file` claims the bytes are actually ISO-8859-1.
    // TODO find a valid way to convert to text
    let bytes = to_bytes(body).await.unwrap();
    let s: String = bytes.iter().map(|&c| c as char).collect();

    Ok(s)
}

fn parse_u8_capture(capture: &Captures, i: usize) -> Option<u32> {
    capture.get(i)
        .map(|m| m.as_str())
        .and_then(|s| s.parse::<u32>().ok())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let https = HttpsConnector::new();
    let client = Client::builder().build::<_, hyper::Body>(https);

    let season_uri: Uri = "https://www.pro-football-reference.com/years/2019/".parse().unwrap();
    let season_page_str = fetch_uri(&client, season_uri).await?.replace("\n<!--", "").replace("\n-->", "");

    let season_page_html = Html::parse_document(&season_page_str);
    let week_uris = parse_season_page(&season_page_html);

    for week_uri in &week_uris[..2] {
        println!("Processing uri {}", week_uri);

        let week_uri_str = week_uri.to_string();

        let year = WEEK_NUM_REGEX.captures(&week_uri_str).and_then(|c| parse_u8_capture(&c, 1)).unwrap();
        let week_num = WEEK_NUM_REGEX.captures(&week_uri_str).and_then(|c| parse_u8_capture(&c, 2)).unwrap();

        let week_page_str = fetch_uri(&client, week_uri.clone()).await?;
        let html_doc = Html::parse_document(&week_page_str);

        let game_log_links = parse_season_week_page(&html_doc);
        let game_logs = join_all(game_log_links.iter().map(|uri| fetch_uri(&client, uri.clone()))).await;

        let game_log_htmls: Vec<Html> = game_logs
            .into_iter()
            .map(|game_log_res| {
                let game_log_html_str = game_log_res.unwrap();

                // TODO a lot of the stats tables below are commented!
                // find a better way to uncomment them
                let s_prime = game_log_html_str.replace("\n<!--", "").replace("\n-->", "");
                Html::parse_document(&s_prime)
            }).collect();

        let game_infos: Vec<GameInfo> = game_log_links
            .iter()
            .zip(game_log_htmls.iter())
            .map(|(game_log_uri, game_log_html)|
                parse_game_log( game_log_uri, &game_log_html)
                    .map(|game_stats| GameInfo { year, week_num, stats: game_stats })
                    .unwrap()
            ).collect();
    }

    Ok(())
}