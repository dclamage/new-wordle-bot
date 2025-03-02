#![allow(clippy::needless_range_loop)]

use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::fs::File;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::{Duration, Instant};

const MAX_GUESS_COUNT: usize = 4;
const SMALL_STATE_SIZE: usize = 32;
type State = SmallVec<[u16; SMALL_STATE_SIZE]>;
const NUM_COLORINGS: usize = 243;
const GREEN: u8 = 2;
const YELLOW: u8 = 1;
const GREY: u8 = 0;

fn main() {
    let answers = read_lines_from_file("answers.txt").expect("Could not read lines from file");
    println!("Answers: {}", answers.len());

    let accepted = read_lines_from_file("accepted.txt").expect("Could not read lines from file");
    println!("Accepted: {}", accepted.len());

    // It is very important that all_words starts with the answers and then the remaining accepted words
    // This is because future code assumes that the first len(answers) indices are the answers
    let all_words = answers
        .iter()
        .chain(accepted.iter())
        .cloned()
        .collect::<Vec<_>>();
    println!("All words: {}", all_words.len());

    let word_coloring_lookup = generate_word_coloring_lookup(&all_words);
    let guess_coloring_lookup =
        generate_guess_coloring_bitfields(&all_words, &answers, &word_coloring_lookup);

    let mut states_per_guess_count: Vec<FxHashMap<State, usize>> = (0..=MAX_GUESS_COUNT)
        .map(|_| FxHashMap::default())
        .collect();

    let initial_state: State = (0..answers.len()).map(|i| i as u16).collect();
    states_per_guess_count[0].insert(initial_state, 0);

    // Track how long it's been since last print
    let start_time = std::time::Instant::now();
    let mut last_print_time = std::time::Instant::now();

    // Open a file for binary writing
    let file = File::create("seen_states.bin").expect("Could not create file");
    let mut writer = io::BufWriter::new(file);
    let mut seen_states_bytes = 0usize;
    let mut seen_states_count = 0usize;
    let mut duplicate_states_count = 0usize;
    let mut duplicate_states_updated = 0usize;
    let mut single_partitions_count = 0usize;
    let mut num_small_states = 0usize;
    let mut num_large_states = 0usize;
    let mut states_processed_since_last_print = 0usize;

    // Write a header with a magic number for identification
    writer
        .write_all(b"WORDLESTATES")
        .expect("Could not write to file");

    for guess_count in 0..MAX_GUESS_COUNT {
        let states_to_process = states_per_guess_count[guess_count]
            .iter()
            .map(|(state, start_index)| (state.clone(), *start_index))
            .collect::<Vec<_>>();

        let mut states_processed_this_count = 0;
        let total_states_this_count = states_to_process.len();

        for (state, start_index) in states_to_process.iter() {
            let start_index = *start_index;
            let next_states = &mut states_per_guess_count[guess_count + 1];

            // Print progress every once in a while
            if last_print_time.elapsed().as_millis() >= 5000 {
                let seen_states_mb = seen_states_bytes as f64 / (1024.0 * 1024.0);
                println!(
                    "[{}s] [{} guesses] [{:.1?}mb] [{:.1?} p/s] [{} processed] States Encountered: {}, Processed: {:.1?}% ({} / {}), Duplicates: {} ({} updated), Single Partitions: {}, SmallVec Efficiency: {:.2?}% (small: {}, large: {})",
                    start_time.elapsed().as_secs(),
                    guess_count,
                    seen_states_mb,
                    states_processed_since_last_print as f64
                        / last_print_time.elapsed().as_secs_f64(),
                    states_processed_since_last_print,
                    seen_states_count,
                    100.0 * states_processed_this_count as f64 / total_states_this_count as f64,
                    states_processed_this_count,
                    total_states_this_count,
                    duplicate_states_count,
                    duplicate_states_updated,
                    single_partitions_count,
                    100.0 * num_small_states as f64 / (num_small_states + num_large_states) as f64,
                    num_small_states,
                    num_large_states
                );
                states_processed_since_last_print = 0;
                last_print_time = std::time::Instant::now();
            }

            for guess_index in start_index..answers.len() {
                // Partitioning
                let mut partitions: SmallVec<[State; 8]> = SmallVec::new();
                for guess_coloring in guess_coloring_lookup[guess_index].iter() {
                    let partition = intersect_states(state, guess_coloring);
                    // We don't really care about states with 1-2 words because the perfect
                    // strategy once reaching those states is obvious.
                    if partition.len() <= 2 {
                        continue;
                    }

                    if partition.len() <= SMALL_STATE_SIZE {
                        num_small_states += 1;
                    } else {
                        num_large_states += 1;
                    }

                    if partition.len() == state.len() {
                        assert!(partitions.is_empty());
                        single_partitions_count += 1;
                        break;
                    }

                    partitions.push(partition);
                }

                for partition in partitions.iter() {
                    if let Some(entry) = next_states.get_mut(partition) {
                        let next_guess_index = guess_index + 1;
                        if next_guess_index < *entry {
                            *entry = next_guess_index;
                            duplicate_states_updated += 1;
                        }
                        duplicate_states_count += 1;
                    } else {
                        next_states.insert(partition.clone(), guess_index);

                        // Write to disk
                        let bytes: &[u8] = bytemuck::cast_slice(partition.as_slice());
                        writer
                            .write_all(&(guess_count as u8).to_le_bytes())
                            .expect("Write error");
                        writer.write_all(bytes).expect("Write error");
                        seen_states_bytes += bytes.len() + 1;

                        // Flush periodically
                        seen_states_count += 1;
                        if seen_states_count % 1000 == 0 {
                            writer.flush().expect("Flush error");
                        }
                    }
                }
            }

            states_processed_this_count += 1;
            states_processed_since_last_print += 1;
        }

        println!(
            "[{}s] Guess count {} completed and added {} states for next guess count",
            start_time.elapsed().as_secs(),
            guess_count,
            states_per_guess_count[guess_count + 1].len()
        );
    }
    println!("Total states: {}", seen_states_count);
}

fn read_lines_from_file<P>(filename: P) -> io::Result<Vec<String>>
where
    P: AsRef<Path>,
{
    let file = File::open(filename)?;
    let reader = io::BufReader::new(file);
    let lines: Vec<String> = reader
        .lines()
        .map(|line| line.expect("Could not read line"))
        .map(|line| line.trim().to_ascii_lowercase())
        .collect();
    Ok(lines)
}

fn calculate_word_colors(answer: &str, guess: &str) -> u8 {
    let answer = answer.as_bytes();
    let guess = guess.as_bytes();
    let len = answer.len();

    let mut num_expected_counts = [0; 26];
    for i in 0..len {
        let answerc = answer[i];
        let guessc = guess[i];
        if answerc != guessc {
            let answer_index = (answerc - b'a') as usize;
            num_expected_counts[answer_index] += 1;
        }
    }

    let mut num_seen_counts = [0; 26];
    let mut colors = 0u8;

    for i in 0..len {
        colors *= 3;

        let guessc = guess[i];
        let answerc = answer[i];
        if !guessc.is_ascii_lowercase() {
            colors += GREY;
            continue;
        }

        if guessc == answerc {
            colors += GREEN;
        } else {
            let guess_index = (guessc - b'a') as usize;
            let num_expected = num_expected_counts[guess_index];
            let num_seen = num_seen_counts[guess_index];
            num_seen_counts[guess_index] += 1;

            if num_seen < num_expected {
                colors += YELLOW;
            } else {
                colors += GREY;
            }
        }
    }

    colors
}

fn generate_word_coloring_lookup(words: &[String]) -> Vec<Vec<u8>> {
    let mut word_coloring_lookup = vec![vec![0; words.len()]; words.len()];
    for (answer_index, answer) in words.iter().enumerate() {
        for (guess_index, guess) in words.iter().enumerate() {
            word_coloring_lookup[answer_index][guess_index] = calculate_word_colors(answer, guess);
        }
    }
    word_coloring_lookup
}

fn generate_guess_coloring_bitfields(
    words: &[String],
    answers: &[String],
    word_coloring_lookup: &[Vec<u8>],
) -> Vec<Vec<State>> {
    let mut guess_coloring_lookup = Vec::with_capacity(words.len());
    for guess_index in 0..words.len() {
        let mut bitsets_by_color = vec![State::default(); NUM_COLORINGS];
        for answer_index in 0..answers.len() {
            let color_index = word_coloring_lookup[answer_index][guess_index] as usize;
            bitsets_by_color[color_index].push(answer_index as u16);
        }
        guess_coloring_lookup.push(
            bitsets_by_color
                .iter()
                .filter(|s| s.len() > 1)
                .cloned()
                .collect(),
        );
    }

    guess_coloring_lookup
}

fn intersect_states(a: &State, b: &State) -> State {
    if a.len() > b.len() {
        return intersect_states(b, a); // Ensure 'a' is always the smaller one
    }

    let mut result = State::new();

    if a.len() <= SMALL_STATE_SIZE {
        for &val in a {
            if b.binary_search(&val).is_ok() {
                result.push(val);
            }
        }
    } else {
        // Otherwise, use two-pointer merging
        let (mut i, mut j) = (0, 0);
        while i < a.len() && j < b.len() {
            match a[i].cmp(&b[j]) {
                std::cmp::Ordering::Equal => {
                    result.push(a[i]);
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
            }
        }
    }

    result
}

struct Profiler {
    start_time: Instant,
    total_duration: Duration,
}

#[allow(dead_code)]
impl Profiler {
    fn new() -> Self {
        Self {
            start_time: Instant::now(),
            total_duration: Duration::ZERO,
        }
    }

    fn start(&mut self) {
        self.start_time = Instant::now()
    }

    fn stop(&mut self) {
        self.total_duration += self.start_time.elapsed();
    }

    fn reset(&mut self) {
        self.total_duration = Duration::ZERO;
    }
}

impl std::fmt::Display for Profiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.total_duration)
    }
}
