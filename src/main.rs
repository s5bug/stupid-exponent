use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use num_format::{Locale, ToFormattedString};
use ratatui::{crossterm, DefaultTerminal, Frame};
use ratatui::crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::crossterm::event::KeyEventKind::Press;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Gauge, Paragraph};

const MODULUS: u64 = 2039; // 2(1019) + 1
const BASE: u64 = 67;
const EXPONENT: u64 = 10_000_000_000;

struct ComputationState {
    result: AtomicU64,
    iterations: AtomicU64,
    park: AtomicBool,
}

fn calculate_result(state: Arc<ComputationState>) {
    // wait for start signal
    while state.park.load(Ordering::Acquire) {
        thread::park();
    }

    let mut iterations: u64 = state.iterations.load(Ordering::Relaxed);
    while iterations < EXPONENT {
        _ = state.result.update(Ordering::Relaxed, Ordering::Relaxed, |r| (r * BASE) % MODULUS);
        iterations = 1 + state.iterations.fetch_add(1, Ordering::Relaxed);
    }
}

fn is_cc(ev: &Event) -> bool {
    match ev {
        Event::Key(kv) if kv.kind == Press => {
            kv.code == KeyCode::Char('c') && kv.modifiers.contains(KeyModifiers::CONTROL)
        }
        _ => false
    }
}

struct App {
    cs: Arc<ComputationState>,
    work_thread: JoinHandle<()>,
    start_time: Instant,
    done_time: Instant,
}

impl App {
    fn current_iters(&self) -> u64 {
        self.cs.iterations.load(Ordering::Relaxed)
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        // first: wait for keypress
        loop {
            terminal.draw(|frame| self.wait_to_start_screen(frame))?;
            let ev = crossterm::event::read()?;
            if is_cc(&ev) {
                return Ok(())
            } else if ev.is_key_press() {
                break;
            }
        }
        // next: unpark the work thread and take note of start time
        self.cs.park.store(false, Ordering::Release);
        self.start_time = Instant::now();
        self.work_thread.thread().unpark();
        loop {
            if self.current_iters() >= EXPONENT {
                break;
            }

            terminal.draw(|frame| self.progress_screen(frame))?;
            if crossterm::event::poll(Duration::from_millis(5))? {
                let event = crossterm::event::read()?;
                if is_cc(&event) { return Ok(()); }
            }
        }
        // we're done, so render the done screen
        self.done_time = Instant::now();
        loop {
            terminal.draw(|frame| self.done_screen(frame))?;
            let ev = crossterm::event::read()?;
            if is_cc(&ev) {
                return Ok(())
            }
        }
    }

    fn wait_to_start_screen(&self, frame: &mut Frame) {
        let block = Block::bordered();
        let p = Paragraph::new("Press any key to start")
            .centered()
            .block(block);

        frame.render_widget(p, frame.area().centered_vertically(Constraint::Length(3)));
    }

    fn progress_screen(&self, frame: &mut Frame) {
        let current_time = Instant::now();
        let elapsed_time = current_time - self.start_time;
        let current_iters = self.current_iters();

        let fracf64 = (current_iters as f64) / (EXPONENT as f64);
        let pctf64 = 100f64 * fracf64;
        let pctu16 = pctf64 as u16;

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Length(1),
                Constraint::Fill(1),
                Constraint::Length(1)
            ])
            .split(frame.area());
        let header = layout[0];
        let bar = layout[1];
        let footer = layout[2];

        let modulus_string = format!("{}", MODULUS);
        let modulus_width = modulus_string.len();
        let result = self.cs.result.load(Ordering::Relaxed);
        let result_p = Paragraph::new(format!("Result: {: >modulus_width$}", result)).centered();
        frame.render_widget(result_p, header);

        let exponent_string = EXPONENT.to_formatted_string(&Locale::en_CA);
        let exponent_width = exponent_string.len();
        let current_iters_string = current_iters.to_formatted_string(&Locale::en_CA);
        let label_string = format!("{: >exponent_width$}/{} ({: >3}%)", current_iters_string, exponent_string, pctu16);

        let gauge = Gauge::default()
            .use_unicode(true)
            .block(Block::bordered().title("Progress"))
            .gauge_style(Color::Red)
            .label(Span::from(label_string)
                .style(Style::new()
                    .bg(Color::Black)
                    .fg(Color::White)))
            .ratio(fracf64);
        frame.render_widget(gauge, bar);

        let total_estimate = if (elapsed_time.as_secs_f64() / fracf64) < 86_400f64 {
            elapsed_time.div_f64(fracf64)
        } else {
            // 48 minute target
            Duration::from_mins(48)
        };
        let remaining_estimate = total_estimate - elapsed_time;
        let elapsed_remaining_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Fill(1), Constraint::Fill(1)])
            .split(footer);

        let elapsed_total_secs = elapsed_time.as_secs();
        let elapsed_minutes = elapsed_total_secs / 60;
        let elapsed_seconds = elapsed_total_secs % 60;
        let elapsed_p = Paragraph::new(format!("Elapsed: {:02}m{:02}s", elapsed_minutes, elapsed_seconds));
        frame.render_widget(elapsed_p, elapsed_remaining_layout[0]);

        let remaining_total_secs = remaining_estimate.as_secs();
        let remaining_minutes = remaining_total_secs / 60;
        let remaining_seconds = remaining_total_secs % 60;
        let remaining_p = Paragraph::new(format!("{:02}m{:02}s Remaining", remaining_minutes, remaining_seconds)).right_aligned();
        frame.render_widget(remaining_p, elapsed_remaining_layout[1]);
    }

    fn done_screen(&self, frame: &mut Frame) {
        let elapsed_time = self.done_time - self.start_time;

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Length(1),
                Constraint::Fill(1),
                Constraint::Length(1)
            ])
            .split(frame.area());
        let header = layout[0];
        let bar = layout[1];
        let footer = layout[2];

        let modulus_string = format!("{}", MODULUS);
        let modulus_width = modulus_string.len();
        let result = self.cs.result.load(Ordering::Acquire);
        let result_p = Paragraph::new(format!("Result: {: >modulus_width$}", result)).centered();
        frame.render_widget(result_p, header);

        let exponent_string = EXPONENT.to_formatted_string(&Locale::en_CA);
        let label_string = format!("{}/{} (100%)", exponent_string, exponent_string);

        let gauge = Gauge::default()
            .use_unicode(true)
            .block(Block::bordered().title("Progress"))
            .gauge_style(Color::Green)
            .label(Span::from(label_string)
                .style(Style::new()
                    .bg(Color::Black)
                    .fg(Color::White)))
            .ratio(1.0f64);
        frame.render_widget(gauge, bar);

        let elapsed_remaining_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Fill(1), Constraint::Fill(1)])
            .split(footer);

        let elapsed_total_secs = elapsed_time.as_secs();
        let elapsed_minutes = elapsed_total_secs / 60;
        let elapsed_seconds = elapsed_total_secs % 60;
        let elapsed_p = Paragraph::new(format!("Elapsed: {:02}m{:02}s", elapsed_minutes, elapsed_seconds));
        frame.render_widget(elapsed_p, elapsed_remaining_layout[0]);

        let remaining_p = Paragraph::new("Done!").right_aligned();
        frame.render_widget(remaining_p, elapsed_remaining_layout[1]);
    }
}

fn main() -> std::io::Result<()> {
    let state = Arc::new(ComputationState {
        iterations: AtomicU64::from(0),
        result: AtomicU64::from(1),
        park: AtomicBool::from(true),
    });

    let work_state = Arc::clone(&state);
    let work_thread = thread::spawn(move || {
        calculate_result(work_state);
    });

    let mut app = App {
        cs: state,
        work_thread,
        // these get immediately discarded so who cares
        start_time: Instant::now(),
        done_time: Instant::now()
    };
    ratatui::run(move |term| app.run(term))?;

    // work_thread.join().expect("could not join work thread");

    Ok(())
}
