//! Terminal formatting utilities for OpenInvar CLI.
//! Uses raw ANSI escape codes — no extra dependencies.
//!
//! This module provides a complete color palette. Not all helpers are used
//! yet — they exist so every command can draw from a consistent set.
#![allow(dead_code)]

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";

const BOLD_CYAN: &str = "\x1b[1;36m";
const BOLD_GREEN: &str = "\x1b[1;32m";
const BOLD_YELLOW: &str = "\x1b[1;33m";
const BOLD_RED: &str = "\x1b[1;31m";
const BOLD_BLUE: &str = "\x1b[1;34m";

// ── public formatting helpers ────────────────────────────────

pub fn banner(subtitle: &str) {
    println!();
    println!("  {BOLD_CYAN}⚡{RESET} {BOLD_CYAN}openinvar{RESET} {DIM}{subtitle}{RESET}");
    println!("  {DIM}───────────────────────────────────────{RESET}");
    println!();
}

pub fn section(title: &str) {
    let pad_len = 40usize.saturating_sub(title.len() + 5);
    let pad = "─".repeat(pad_len);
    println!();
    println!("  {DIM}───{RESET} {BOLD}{title}{RESET} {DIM}{pad}{RESET}");
    println!();
}

pub fn success(msg: &str) {
    println!("  {GREEN}✓{RESET} {msg}");
}

pub fn info(msg: &str) {
    println!("  {CYAN}›{RESET} {msg}");
}

pub fn warning(msg: &str) {
    println!("  {YELLOW}⚠{RESET}  {BOLD_YELLOW}{msg}{RESET}");
}

pub fn error(msg: &str) {
    eprintln!("  {RED}✗{RESET} {BOLD_RED}{msg}{RESET}");
}

pub fn stat(label: &str, value: &str) {
    println!("  {DIM}{label:<18}{RESET} {value}");
}

pub fn stat_highlight(label: &str, value: &str) {
    println!("  {label:<18} {BOLD}{value}{RESET}");
}

pub fn dim_line(msg: &str) {
    println!("  {DIM}{msg}{RESET}");
}

pub fn blank() {
    println!();
}

// ── inline formatters — return styled strings ────────────────

pub fn bold(s: &str) -> String {
    format!("{BOLD}{s}{RESET}")
}

pub fn dim(s: &str) -> String {
    format!("{DIM}{s}{RESET}")
}

pub fn cyan(s: &str) -> String {
    format!("{CYAN}{s}{RESET}")
}

pub fn green(s: &str) -> String {
    format!("{GREEN}{s}{RESET}")
}

pub fn yellow(s: &str) -> String {
    format!("{YELLOW}{s}{RESET}")
}

pub fn red(s: &str) -> String {
    format!("{RED}{s}{RESET}")
}

pub fn blue(s: &str) -> String {
    format!("{BLUE}{s}{RESET}")
}

pub fn magenta(s: &str) -> String {
    format!("{MAGENTA}{s}{RESET}")
}

pub fn bold_cyan(s: &str) -> String {
    format!("{BOLD_CYAN}{s}{RESET}")
}

pub fn bold_green(s: &str) -> String {
    format!("{BOLD_GREEN}{s}{RESET}")
}

pub fn bold_yellow(s: &str) -> String {
    format!("{BOLD_YELLOW}{s}{RESET}")
}

pub fn bold_blue(s: &str) -> String {
    format!("{BOLD_BLUE}{s}{RESET}")
}

pub fn bold_red(s: &str) -> String {
    format!("{BOLD_RED}{s}{RESET}")
}

// ── progress / timing ────────────────────────────────────────

pub fn step(label: &str, detail: &str) {
    println!("  {DIM}›{RESET} {label:<22} {DIM}{detail}{RESET}");
}

pub fn done(msg: &str) {
    println!();
    println!("  {BOLD_GREEN}✓{RESET} {msg}");
    println!();
}

pub fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hours = (now % 86400) / 3600;
    let minutes = (now % 3600) / 60;
    let seconds = now % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub fn file_path(path: &str) -> String {
    format!("{BOLD_BLUE}{path}{RESET}")
}

pub fn symbol_name(name: &str) -> String {
    format!("{BOLD}{name}{RESET}")
}

pub fn alias_tag(alias: &str) -> String {
    format!("{BOLD_YELLOW}{alias}{RESET}")
}

pub fn property_name(prop: &str) -> String {
    format!("{MAGENTA}.{prop}{RESET}")
}
