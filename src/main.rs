use clap::arg_enum;
use futures::future::join_all;
use hyper::{
    http::Uri,
    body::{to_bytes},
    Client, client::HttpConnector
};
use hyper_tls::HttpsConnector;
use structopt::StructOpt;
use tokio::fs::create_dir_all;
use tokio::fs::File;

use std::{error::Error, collections::{HashMap, BTreeMap}, fmt};
use scraper::{Html, Selector, Node, ElementRef};
use regex::{Regex, Captures};
use lazy_static::lazy_static;
use csv_async::AsyncWriter;

lazy_static!{
    static ref GAME_ID_REGEX: Regex = Regex::new(r".*/(\w+)\.htm").unwrap();
    static ref WEEK_NUM_REGEX: Regex = Regex::new(r".*/(\d{4})/week_(\d{1,2})\.htm").unwrap();
    static ref PLAYER_YEARS_REGEX: Regex = Regex::new(r"(\d{4})\-(\d{4})").unwrap();
    static ref PLAYER_POS_REGEX: Regex = Regex::new(r"\((.*)\)").unwrap();
    static ref PLAYER_ID_REGEX: Regex = Regex::new(r".*/(.+)\.htm").unwrap();
}
const PFR_DOMAIN: &str = "https://www.pro-football-reference.com";

#[derive(Debug,Clone,Copy,PartialEq,Eq,Hash)]
enum StatsType {
    Offense,
    Defense,
    Returns,
    Kicking,
    AdvPassing,
    AdvRushing,
    AdvReceiving,
    AdvDefense
}

impl fmt::Display for StatsType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug)]
struct TypedStats<'a> {
    stats_type: StatsType,
    stats: BTreeMap<&'a str, &'a str>
}

#[derive(Debug)]
struct PlayerGameStats<'a> {
    player_id: &'a str,
    player_name: &'a str,
    typed_stats: TypedStats<'a>
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

fn parse_year(week_uri: &str) -> u32 {
    WEEK_NUM_REGEX.captures(week_uri).and_then(|c| parse_u32_capture(&c, 1)).unwrap()
}

#[test]
fn parse_year_test() {
    assert_eq!(2021, parse_year("https://www.pro-football-reference.com/years/2021/week_1.htm"));
    assert_eq!(2021, parse_year("https://www.pro-football-reference.com/years/2021/week_10.htm"));
    assert_eq!(2021, parse_year("https://www.pro-football-reference.com/years/2021/week_17.htm"));

    assert_eq!(2019, parse_year("https://www.pro-football-reference.com/years/2019/week_1.htm"));
    assert_eq!(2019, parse_year("https://www.pro-football-reference.com/years/2019/week_10.htm"));
    assert_eq!(2019, parse_year("https://www.pro-football-reference.com/years/2019/week_17.htm"));
}

fn parse_week_num(week_uri: &str) -> u32 {
    WEEK_NUM_REGEX.captures(week_uri).and_then(|c| parse_u32_capture(&c, 2)).unwrap()
}

#[test]
fn parse_week_num_test() {
    assert_eq!(1, parse_week_num("https://www.pro-football-reference.com/years/2021/week_1.htm"));
    assert_eq!(10, parse_week_num("https://www.pro-football-reference.com/years/2021/week_10.htm"));
    assert_eq!(17, parse_week_num("https://www.pro-football-reference.com/years/2021/week_17.htm"));
}

fn selector(selector_str: &str) -> Selector {
    Selector::parse(selector_str).unwrap()
}

fn parse_a_href(elt_ref: ElementRef) -> Uri {
    elt_ref.value()
        .attr("href")
        .and_then(|link| {
            if link.starts_with('/') {
                format!("{}{}", PFR_DOMAIN, link).parse().ok()
            } else {
                link.parse().ok()
            }
        }).unwrap()
}

fn parse_player_stats_table<'a>(html_doc: &'a Html, table_selector_str: &str, stats_type: StatsType) -> Result<Vec<PlayerGameStats<'a>>, String> {
    let table_selector = selector(table_selector_str);
    let table_elt = match html_doc.select(&table_selector).next() {
        Some(elt) => elt,
        None => return Err(format!("no table element for selector '{}' found", table_selector_str))
    };

    let tbody_selector= selector("tbody");
    let table_body = table_elt.select(&tbody_selector).next().unwrap();

    let mut retlist: Vec<PlayerGameStats> = vec![];
    'rowlabel: for table_row in table_body.children() {

        let mut player_data: BTreeMap<&str, &str> = BTreeMap::new();

        for table_data_noderef in table_row.children() {
            let table_data_elt = match table_data_noderef.value() {
                Node::Element(elt) => elt,
                _ => continue
            };

            // Handle player id.

            if let Some(id) = table_data_elt.attr("data-append-csv") {
                player_data.insert("player_id", id);
            }

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

            if let Some(Node::Text(t)) = data_val_elt {
                player_data.insert(stat_name.trim(), t.text.as_ref().trim());
            }
        }

        if let (Some(&player_id), Some(&player_name)) = (player_data.get("player_id"), player_data.get("player")) {
            retlist.push(PlayerGameStats {
                player_id,
                player_name,
                typed_stats: TypedStats {
                    stats_type,
                    stats: player_data
                }
            });
        }
    }

    Ok(retlist)
}

fn parse_game_log(game_log_html: &Html) -> Result<GameStats, String> {
    let mut player_stats: Vec<PlayerGameStats> = vec![];

    let game_link_selector = selector("link[rel=canonical]");
    let game_link = game_log_html.select(&game_link_selector).next().unwrap().value().attr("href").unwrap();
    let game_id = GAME_ID_REGEX.captures(game_link).unwrap().get(1).unwrap().as_str();

    // Mandatory stats.
    player_stats.extend(parse_player_stats_table(game_log_html, "#player_offense", StatsType::Offense)?);
    player_stats.extend(parse_player_stats_table(game_log_html, "#player_defense", StatsType::Defense)?);
    player_stats.extend(parse_player_stats_table(game_log_html, "#returns", StatsType::Returns)?);
    player_stats.extend(parse_player_stats_table(game_log_html, "#kicking", StatsType::Kicking)?);

    // Optional advanced stats.
    let adv_passing = parse_player_stats_table(game_log_html, "#passing_advanced", StatsType::AdvPassing);
    let adv_rushing = parse_player_stats_table( game_log_html, "#rushing_advanced", StatsType::AdvRushing);
    let adv_receiving = parse_player_stats_table(game_log_html, "#receiving_advanced", StatsType::AdvReceiving);
    let adv_def = parse_player_stats_table(game_log_html, "#defense_advanced", StatsType::AdvDefense);

    for adv_stats in [adv_passing, adv_rushing, adv_receiving, adv_def] {
        if let Ok(stats) = adv_stats {
            player_stats.extend(stats);
        }
    }

    Ok(GameStats { game_id, player_stats })
}

fn parse_season_week_page(season_week_log_html: & Html) -> Vec<Uri> {
    season_week_log_html
        .select(&selector(".gamelink a"))
        .filter(|elt_ref| {
            let text_opt = elt_ref
                .first_child()
                .map(|noderef| match noderef.value() {
                    Node::Text(t) => t.to_string() == "F",
                    _ => false
                });

            text_opt.unwrap_or(false)
        }).map(parse_a_href)
        .collect()
}

fn parse_season_page(season_week_log_html: &Html) -> Vec<Uri> {
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
    let resp = client.get(uri).await?;

    let body = resp.into_body();
    // NOTE response header says it is UTF-8 but UTF-8 parsing fails...
    // `file` claims the bytes are actually ISO-8859-1.
    // TODO find a valid way to convert to text
    let bytes = to_bytes(body).await.unwrap();
    let s: String = bytes.iter().map(|&c| c as char).collect();

    Ok(s)
}

fn parse_u32_capture(capture: &Captures, i: usize) -> Option<u32> {
    capture.get(i)
        .map(|m| m.as_str())
        .and_then(|s| s.parse::<u32>().ok())
}

async fn process_week(client: &Client<HttpsConnector<HttpConnector>>, week_uri: &Uri, base_output_dir: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let week_uri_str = week_uri.to_string();

    let year = parse_year(&week_uri_str);
    let week_num = parse_week_num(&week_uri_str);

    let week_page_str = fetch_uri(client, week_uri.clone()).await?;
    let html_doc = Html::parse_document(&week_page_str);

    let game_log_links = parse_season_week_page(&html_doc);
    let game_logs = join_all(game_log_links.iter().map(|uri| fetch_uri(client, uri.clone()))).await;

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
            parse_game_log(game_log_html)
                .map(|game_stats| GameInfo { year, week_num, stats: game_stats })
                .unwrap_or_else(|_| panic!("Failed to parse game log at {}", game_log_uri))
        ).collect();

    let all_typed_stats = game_infos.iter()
        .flat_map(|game_info| game_info.stats.player_stats.iter())
        .map(|player_game_stats| &player_game_stats.typed_stats);

    let mut stats_type_cols: HashMap<StatsType, Vec<&str>> = HashMap::new();
    for typed_stats in all_typed_stats {
        stats_type_cols.entry(typed_stats.stats_type)
            .or_insert_with(|| typed_stats.stats.keys().copied().collect());
    }

    // Write output files
    let dir_name = format!("{}/{}/{}", base_output_dir, year, week_num);
    create_dir_all(&dir_name).await?;

    let mut stats_type_writer: HashMap<StatsType, AsyncWriter<File>> = HashMap::new();
    for (stats_type, cols) in stats_type_cols.iter() {
        let stats_type_f = File::create(format!("{}/{}", &dir_name, stats_type)).await?;

        let mut all_cols = vec!["year", "week", "game_id"];
        all_cols.extend(cols);

        let mut csv_writer = AsyncWriter::from_writer(stats_type_f);
        csv_writer.write_record(all_cols).await?;

        stats_type_writer.insert(*stats_type, csv_writer);
    }

    for game_info in game_infos {
        for player_stats in game_info.stats.player_stats {
            let TypedStats { stats_type, stats} = player_stats.typed_stats;

            let stats_writer = stats_type_writer.get_mut(&stats_type).unwrap();

            let year: &str = &game_info.year.to_string();
            let week_num: &str = &game_info.week_num.to_string();
            let game_id: &str = game_info.stats.game_id;

            let mut vals: Vec<&str> = vec![year, week_num, game_id];
            for &col in stats_type_cols.get(&stats_type).unwrap() {
                let stat_val = match stats.get(col) {
                    Some(&s) => s,
                    None => ""
                };

                vals.push(stat_val);
            }

            stats_writer.write_record(vals).await?;
        }
    }
    
    println!("Finished processing {} week {}", year, week_num);
    Ok(())
}

async fn process_year(client: &Client<HttpsConnector<HttpConnector>>, year: u32, output_dir: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    println!("Fetching data for {}", year);

    let season_uri: Uri = format!("https://www.pro-football-reference.com/years/{}/", year).parse().unwrap();
    let season_page_str = fetch_uri(client, season_uri).await?.replace("\n<!--", "").replace("\n-->", "");

    let season_page_html = Html::parse_document(&season_page_str);
    let week_uris = parse_season_page(&season_page_html);

    let process_week_futures = week_uris.iter().map(|week_uri| process_week(client, week_uri, output_dir));
    let processed_weeks_res: Result<(), Box<dyn Error + Send + Sync>> = join_all(process_week_futures).await.into_iter().collect();

    processed_weeks_res?;

    Ok(())
}

async fn process_players(client: &Client<HttpsConnector<HttpConnector>>, output_dir: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    println!("Processing player roster");

    let players_uri: Uri = "https://www.pro-football-reference.com/players/".parse().unwrap();
    let players_page_str = fetch_uri(client, players_uri).await?;
    let players_page_html = Html::parse_document(&players_page_str);

    let letter_links: Vec<Uri> = players_page_html
        .select(&selector("ul.page_index li>a"))
        .map(parse_a_href)
        .collect();

    let players_f = File::create(format!("{}/players", output_dir)).await?;
    let mut csv_writer = AsyncWriter::from_writer(players_f);
    csv_writer.write_record([
        "player_id",
        "player_name",
        "positions",
        "begin_year",
        "end_year"
    ]).await?;

    for letter_link in letter_links {
        println!("Downloading player data from {}", letter_link);

        let letter_page_str = fetch_uri(client, letter_link).await?;
        let letter_page = Html::parse_document(&letter_page_str);

        let player_texts_selector = selector("#div_players p");
        let player_texts = letter_page.select(&player_texts_selector);
        for player_text in player_texts {
            let mut player_vals: Vec<&str> = vec![];
            let mut is_active = false;
            for child in player_text.children() {
                match child.value() {
                    Node::Element(elt) => {
                        let is_bolded = elt.attr("href").is_none();
                        is_active = is_bolded;

                        let anchor_node = if !is_bolded {
                            child
                        } else {
                            child.first_child().unwrap()
                        };

                        let player_name = match anchor_node.first_child().unwrap().value() {
                            Node::Text(player_name_text) => player_name_text.as_ref(),
                            _ => ""
                        }.trim();

                        let player_id = match anchor_node.value().as_element().unwrap().attr("href") {
                            Some(href) => 
                                PLAYER_ID_REGEX.captures(href)
                                .and_then(|captures| captures.get(1))
                                .unwrap()
                                .as_str(),
                            None => continue
                        };

                        player_vals.push(player_id);
                        player_vals.push(player_name);

                        if is_bolded {
                            let position = child.first_child()
                                .and_then(|c| c.next_sibling())
                                .and_then(|position_node| match position_node.value() {
                                    Node::Text(pos_text) => PLAYER_POS_REGEX.captures(pos_text.as_ref()),
                                    _ => None
                                }).and_then(|captures| captures.get(1))
                                .map(|m| m.as_str())
                                .unwrap()
                                .trim();

                            player_vals.push(position);
                        }
                    },
                    Node::Text(desc_text) => {
                        let desc: &str = desc_text.as_ref();

                        // Need to parse position out still.
                        let position_opt = PLAYER_POS_REGEX.captures(desc)
                            .and_then(|captures| captures.get(1))
                            .map(|m| m.as_str());

                        if let Some(position) = position_opt {
                            player_vals.push(position);
                        }
                            
                        let captures = PLAYER_YEARS_REGEX.captures(desc_text.as_ref()).unwrap();

                        let begin_year = captures.get(1).unwrap().as_str().trim();
                        let end_year = if is_active {
                            ""
                        } else {
                            captures.get(2).unwrap().as_str().trim()
                        };

                        player_vals.push(begin_year);
                        player_vals.push(end_year);
                    },
                    _ =>  ()
                }
            }

            csv_writer.write_record(player_vals).await?;
        }
    }

    Ok(())
}

arg_enum! {
    #[derive(Debug)]
    enum Mode {
        Player,
        Game
    }
}

#[derive(Debug, StructOpt)]
struct Opts {

    /// Whether to download player roster or game data.
    #[structopt(
        short,
        long,
        possible_values = &Mode::variants(),
        case_insensitive = true,
        default_value = "game"
    )]
    mode: Mode,

    /// Year to download game level stats for.
    #[structopt(short, long, required_if("mode", "game"))]
    year: Option<u32>,

    /// Output directory to write data files to.
    // TODO use Path or PathBuf?
    #[structopt(short, long, default_value = "output")]
    output_dir: String
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let Opts { mode, year, output_dir } = Opts::from_args();

    let https = HttpsConnector::new();
    let client = Client::builder().build::<_, hyper::Body>(https);

    match mode {
        Mode::Player => {
            process_players(&client, &output_dir).await
        }
        Mode::Game => {
            process_year(&client, year.unwrap(), &output_dir).await
        }
    }
}