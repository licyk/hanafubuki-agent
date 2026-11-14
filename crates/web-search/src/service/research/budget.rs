// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use crate::WebSearchError;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub(crate) struct SearchBudget {
    requests: AtomicUsize,
    pub partial: parking_lot::Mutex<Vec<(u8, crate::WebSearchResponse)>>,
    pub round: AtomicUsize,
    firecrawl: AtomicUsize,
    crawl4ai: AtomicUsize,
    fetch_attempts: AtomicUsize,
    fetch_limit: usize,
    pub deadline: Instant,
}
impl SearchBudget {
    pub fn new(depth: usize) -> Self {
        Self {
            requests: AtomicUsize::new(0),
            partial: parking_lot::Mutex::new(Vec::new()),
            round: AtomicUsize::new(1),
            firecrawl: AtomicUsize::new(0),
            crawl4ai: AtomicUsize::new(0),
            fetch_attempts: AtomicUsize::new(0),
            fetch_limit: depth * 2 + 4,
            deadline: Instant::now() + Duration::from_secs(90),
        }
    }
    pub fn charge_fetch(&self) -> Result<(), crate::WebFetchError> {
        if Instant::now() >= self.deadline {
            return Err(crate::WebFetchError::Timeout);
        }
        self.fetch_attempts
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < self.fetch_limit).then_some(n + 1)
            })
            .map(|_| ())
            .map_err(|_| crate::WebFetchError::BudgetExhausted)
    }
    pub fn charge_reader(
        &self,
        backend: crate::WebFetchBackend,
    ) -> Result<(), crate::WebFetchError> {
        let (counter, limit) = match backend {
            crate::WebFetchBackend::Firecrawl => (&self.firecrawl, 3),
            crate::WebFetchBackend::Crawl4ai => (&self.crawl4ai, 2),
            _ => return Ok(()),
        };
        counter
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < limit).then_some(n + 1)
            })
            .map(|_| ())
            .map_err(|_| crate::WebFetchError::BudgetExhausted)
    }
    pub fn fetch_exhausted(&self) -> bool {
        self.fetch_attempts.load(Ordering::Relaxed) >= self.fetch_limit
    }
    pub fn charge(&self) -> Result<(), WebSearchError> {
        if Instant::now() >= self.deadline {
            return Err(WebSearchError::Timeout);
        }
        self.requests
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < 12).then_some(n + 1)
            })
            .map(|_| ())
            .map_err(|_| WebSearchError::BudgetExhausted)
    }
    pub fn used(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }
}
