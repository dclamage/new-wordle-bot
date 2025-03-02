//use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use std::fs::File;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const GREEN: u8 = 2;
const YELLOW: u8 = 1;
const GREY: u8 = 0;

fn main() {
    // Create a thread pool with a maximum of 4 threads.
    //let pool = ThreadPoolBuilder::new().num_threads(10).build().unwrap();
    //pool.install(|| {
        rayon_main();
    //});
}

fn rayon_main() {
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

    let initial_state = (0..answers.len() as u16).collect::<Vec<u16>>();

    let mut states_queue = vec![initial_state.clone()];
    let seen_states_by_ones_count: Arc<Vec<RwLock<FxHashSet<Vec<u16>>>>> =
        Arc::new((0..=answers.len()).map(|_| RwLock::new(FxHashSet::default())).collect());
    seen_states_by_ones_count[initial_state.len()].write().unwrap().insert(initial_state);

    // Track how long it's been since last print
    let start_time = std::time::Instant::now();
    let mut last_print_time = std::time::Instant::now();

    // Open a file for binary writing
    let file = File::create("seen_states.bin").expect("Could not create file");
    let mut writer = io::BufWriter::new(file);
    let seen_states_bytes = AtomicUsize::new(0);
    let seen_states_count = AtomicUsize::new(0);
    let trivial_duplicate_states_count = AtomicUsize::new(0);
    let duplicate_states_count = AtomicUsize::new(0);
    let single_partitions_count = AtomicUsize::new(0);
    let two_answers_count = AtomicUsize::new(0);
    let total_states_processed = AtomicUsize::new(0);
    //let mut total_prints = 0usize;

    // Write a header with a magic number for identification
    writer
        .write_all(b"WORDLESTATES")
        .expect("Could not write to file");

    while let Some(state) = states_queue.pop() {
        total_states_processed.fetch_add(1, Ordering::Relaxed);

        // Print progress every once in a while
        if last_print_time.elapsed().as_millis() >= 5000 {
            let seen_states_mb =
                seen_states_bytes.load(Ordering::Relaxed) as f64 / (1024.0 * 1024.0);
            println!(
                "[{}s] [{:.1?}mb] States Encountered: {} (Processed: {}, Queue: {}), Duplicates: {} + {}, Single Partitions: {}, Two Answers: {}",
                start_time.elapsed().as_secs(),
                seen_states_mb,
                seen_states_count.load(Ordering::Relaxed),
                total_states_processed.load(Ordering::Relaxed),
                states_queue.len(),
                trivial_duplicate_states_count.load(Ordering::Relaxed),
                duplicate_states_count.load(Ordering::Relaxed),
                single_partitions_count.load(Ordering::Relaxed),
                two_answers_count.load(Ordering::Relaxed)
            );

            // Print histogram of number of states per ones count
            let histogram = seen_states_by_ones_count
                .iter()
                .map(|set| set.read().unwrap().len())
                .collect::<Vec<_>>();
            let mut bucket_counts = vec![0; 10];
            for (index, &count) in histogram.iter().enumerate() {
                if count == 0 {
                    continue;
                }
                let bucket_index = (index as f64).log2().floor() as usize;
                if bucket_index >= bucket_counts.len() {
                    bucket_counts.resize(bucket_index + 1, 0);
                }
                bucket_counts[bucket_index] += count;
            }

            for (i, &count) in bucket_counts.iter().enumerate() {
                if count == 0 {
                    continue;
                }
                let range_start = 2_usize.pow(i as u32);
                let range_end = 2_usize.pow(i as u32 + 1) - 1;
                println!("{:>5} - {:>5}: {}", range_start, range_end, count);
            }
            println!();

            // Reset the last print time and increment the total prints
            last_print_time = std::time::Instant::now();
            //total_prints += 1;

            // Useful to reset the profilers after some time because the initial states may
            // have a different timing profile
            /*if total_prints == 5 {
                // Reset profilers
                prepass_profiler.reset();
                create_partitions_profiler.reset();
                fill_partitions_profiler.reset();
                add_partitions_profiler.reset();
                add_partitions_contains_profiler.reset();
            }*/
        }

        let state_arc = Arc::new(state.clone()); // Share state across threads safely

        let local_partitions: Vec<Vec<u16>> = (0..all_words.len())
            .into_par_iter()
            .filter_map(|guess_index| {
                let state = Arc::clone(&state_arc);

                // Prepass: Filter out trivial cases
                let mut seen_colors = [false; 243];
                let mut unique_colors = Vec::with_capacity(243);
                for &answer_index in state.iter() {
                    let answer_colors = word_coloring_lookup[answer_index as usize][guess_index];
                    if !seen_colors[answer_colors as usize] {
                        seen_colors[answer_colors as usize] = true;
                        unique_colors.push(answer_colors);
                    }
                }
                if unique_colors.len() <= 1 {
                    single_partitions_count.fetch_add(1, Ordering::Relaxed);
                    return None; // Skip processing this guess
                }

                // Partitioning
                let mut partitions: FxHashMap<u8, Vec<u16>> = FxHashMap::default();
                for &answer_index in state.iter() {
                    let answer_colors = word_coloring_lookup[answer_index as usize][guess_index];
                    partitions
                        .entry(answer_colors)
                        .or_default()
                        .push(answer_index);
                }

                // Collect all valid partitions (avoid modifying global structures)
                let mut valid_partitions = Vec::new();
                for partition in partitions.values() {
                    let count = partition.len();
                    if count <= 1 {
                        trivial_duplicate_states_count.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }

                    if seen_states_by_ones_count[count].read().unwrap().contains(partition) {
                        duplicate_states_count.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }

                    valid_partitions.push(partition.clone()); // Collect for single-threaded processing
                }
                Some(valid_partitions)
            })
            .flatten()
            .collect(); // Collect all valid partitions from threads

        // Single-threaded processing phase:
        for partition in local_partitions {
            let count = partition.len();

            // Insert into global deduplication set
            if !seen_states_by_ones_count[count].write().unwrap().insert(partition.clone()) {
                duplicate_states_count.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            // Add new state to the queue
            if count > 2 {
                states_queue.push(partition.clone());
            } else {
                two_answers_count.fetch_add(1, Ordering::Relaxed);
            }

            // Write to file in a controlled manner
            let bytes: &[u8] = bytemuck::cast_slice(&partition);
            writer
                .write_all(&(partition.len() as u16).to_le_bytes())
                .expect("Write error");
            writer.write_all(bytes).expect("Write error");
            seen_states_bytes.fetch_add(bytes.len(), Ordering::Relaxed);

            // Flush periodically
            if seen_states_count.fetch_add(1, Ordering::Relaxed) % 1000 == 0 {
                writer.flush().expect("Flush error");
            }
        }
    }
    println!(
        "Total states: {}",
        seen_states_count.load(Ordering::Relaxed)
    );
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
