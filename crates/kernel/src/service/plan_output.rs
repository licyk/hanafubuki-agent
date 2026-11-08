const OPEN_TAG: &str = "<proposed_plan>";
const CLOSE_TAG: &str = "</proposed_plan>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PlanOutputSegment {
    Normal(String),
    PlanStart,
    PlanDelta(String),
    PlanEnd,
}

#[derive(Clone, Debug, Default)]
pub(super) struct PlanOutputProjection {
    parser: PlanOutputParser,
    visible_text: String,
    plan_text: String,
    plan_open: bool,
    closed_plan_count: usize,
    invalid: bool,
}

impl PlanOutputProjection {
    pub(super) fn push_str(&mut self, delta: &str) -> Vec<PlanOutputSegment> {
        let segments = self.parser.push_str(delta);
        self.observe(&segments);
        segments
    }

    pub(super) fn finish(&mut self) -> Vec<PlanOutputSegment> {
        let segments = self.parser.finish();
        self.observe(&segments);
        segments
    }

    pub(super) fn visible_text(&self) -> &str {
        &self.visible_text
    }

    pub(super) fn proposed_plan(&self) -> Option<&str> {
        (self.closed_plan_count == 1
            && !self.plan_open
            && !self.invalid
            && !self.plan_text.trim().is_empty())
        .then_some(self.plan_text.as_str())
    }

    fn observe(&mut self, segments: &[PlanOutputSegment]) {
        for segment in segments {
            match segment {
                PlanOutputSegment::Normal(delta) => self.visible_text.push_str(delta),
                PlanOutputSegment::PlanStart => {
                    if self.plan_open || self.closed_plan_count > 0 {
                        self.invalid = true;
                    }
                    self.plan_open = true;
                }
                PlanOutputSegment::PlanDelta(delta) => {
                    if self.closed_plan_count == 0 {
                        self.plan_text.push_str(delta);
                    }
                }
                PlanOutputSegment::PlanEnd => {
                    if !self.plan_open {
                        self.invalid = true;
                    }
                    self.plan_open = false;
                    self.closed_plan_count = self.closed_plan_count.saturating_add(1);
                    if self.closed_plan_count > 1 {
                        self.invalid = true;
                    }
                }
            }
        }
    }
}

/// Incrementally separates exact line-delimited proposed-plan blocks from
/// ordinary assistant text. A started block is deliberately not closed by
/// `finish`; callers may only treat an observed `PlanEnd` as implementable.
#[derive(Clone, Debug)]
pub(super) struct PlanOutputParser {
    active: bool,
    detect_tag: bool,
    line_buffer: String,
}

impl PlanOutputParser {
    pub(super) fn new() -> Self {
        Self {
            active: false,
            detect_tag: true,
            line_buffer: String::new(),
        }
    }

    pub(super) fn push_str(&mut self, delta: &str) -> Vec<PlanOutputSegment> {
        let mut segments = Vec::new();
        let mut run = String::new();

        for ch in delta.chars() {
            if self.detect_tag {
                if !run.is_empty() {
                    self.push_text(std::mem::take(&mut run), &mut segments);
                }
                self.line_buffer.push(ch);
                if ch == '\n' {
                    self.finish_line(&mut segments);
                    continue;
                }
                let slug = self.line_buffer.trim_start();
                if slug.is_empty() || self.is_relevant_tag_prefix(slug) {
                    continue;
                }
                let buffered = std::mem::take(&mut self.line_buffer);
                self.detect_tag = false;
                self.push_text(buffered, &mut segments);
                continue;
            }

            run.push(ch);
            if ch == '\n' {
                self.push_text(std::mem::take(&mut run), &mut segments);
                self.detect_tag = true;
            }
        }

        if !run.is_empty() {
            self.push_text(run, &mut segments);
        }
        segments
    }

    pub(super) fn finish(&mut self) -> Vec<PlanOutputSegment> {
        let mut segments = Vec::new();
        if !self.line_buffer.is_empty() {
            let buffered = std::mem::take(&mut self.line_buffer);
            let slug = buffered.trim_start().trim_end();
            if slug == OPEN_TAG && !self.active {
                push_segment(&mut segments, PlanOutputSegment::PlanStart);
                self.active = true;
            } else if slug == CLOSE_TAG && self.active {
                push_segment(&mut segments, PlanOutputSegment::PlanEnd);
                self.active = false;
            } else {
                self.push_text(buffered, &mut segments);
            }
        }
        self.detect_tag = true;
        segments
    }

    fn finish_line(&mut self, segments: &mut Vec<PlanOutputSegment>) {
        let line = std::mem::take(&mut self.line_buffer);
        let slug = line
            .strip_suffix('\n')
            .unwrap_or(&line)
            .trim_start()
            .trim_end();

        if slug == OPEN_TAG && !self.active {
            push_segment(segments, PlanOutputSegment::PlanStart);
            self.active = true;
        } else if slug == CLOSE_TAG && self.active {
            push_segment(segments, PlanOutputSegment::PlanEnd);
            self.active = false;
        } else {
            self.push_text(line, segments);
        }
        self.detect_tag = true;
    }

    fn is_relevant_tag_prefix(&self, slug: &str) -> bool {
        let slug = slug.trim_end();
        if self.active {
            CLOSE_TAG.starts_with(slug)
        } else {
            OPEN_TAG.starts_with(slug) || CLOSE_TAG.starts_with(slug)
        }
    }

    fn push_text(&self, text: String, segments: &mut Vec<PlanOutputSegment>) {
        if self.active {
            push_segment(segments, PlanOutputSegment::PlanDelta(text));
        } else {
            push_segment(segments, PlanOutputSegment::Normal(text));
        }
    }
}

impl Default for PlanOutputParser {
    fn default() -> Self {
        Self::new()
    }
}

fn push_segment(segments: &mut Vec<PlanOutputSegment>, segment: PlanOutputSegment) {
    match segment {
        PlanOutputSegment::Normal(delta) => {
            if delta.is_empty() {
                return;
            }
            if let Some(PlanOutputSegment::Normal(existing)) = segments.last_mut() {
                existing.push_str(&delta);
            } else {
                segments.push(PlanOutputSegment::Normal(delta));
            }
        }
        PlanOutputSegment::PlanDelta(delta) => {
            if delta.is_empty() {
                return;
            }
            if let Some(PlanOutputSegment::PlanDelta(existing)) = segments.last_mut() {
                existing.push_str(&delta);
            } else {
                segments.push(PlanOutputSegment::PlanDelta(delta));
            }
        }
        PlanOutputSegment::PlanStart => segments.push(PlanOutputSegment::PlanStart),
        PlanOutputSegment::PlanEnd => segments.push(PlanOutputSegment::PlanEnd),
    }
}

#[cfg(test)]
mod tests;
