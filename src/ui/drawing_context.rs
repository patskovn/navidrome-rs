use std::cell::RefCell;

use ratatui::Frame;
use ratatui::layout::Rect;

type DrawOnce<'b> = Box<dyn FnOnce(&mut Frame<'b>, Rect) + 'b>;

/// Storage for deferred modal draw operations.
/// Modals are drawn after the main render tree has been processed.
pub struct ModalQueue<'b> {
    modals: RefCell<Vec<DrawOnce<'b>>>,
    root_area: Rect,
}

impl<'b> ModalQueue<'b> {
    pub fn new(root_area: Rect) -> Self {
        Self {
            modals: RefCell::new(Vec::new()),
            root_area,
        }
    }
}

/// A context for drawing UI components.
///
/// This struct holds a mutable reference to the frame and the current drawing area.
/// It can create child contexts with different areas while properly handling the borrow checker.
/// It also supports deferred modal rendering via the modal queue.
pub struct DrawingContext<'a, 'b, 'q> {
    frame: &'a mut Frame<'b>,
    area: Rect,
    queue: &'q ModalQueue<'b>,
}

impl<'a, 'b, 'q> DrawingContext<'a, 'b, 'q> {
    pub fn new(frame: &'a mut Frame<'b>, queue: &'q ModalQueue<'b>) -> Self {
        Self {
            frame,
            area: queue.root_area,
            queue,
        }
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    #[allow(dead_code)]
    pub fn root_area(&self) -> Rect {
        self.queue.root_area
    }

    pub fn frame(&mut self) -> &mut Frame<'b> {
        self.frame
    }

    /// Create a child context with a different area.
    ///
    /// This reborrows the frame, so the parent context cannot be used
    /// until the child context is dropped.
    pub fn with_area(&mut self, area: Rect) -> DrawingContext<'_, 'b, 'q> {
        DrawingContext {
            frame: self.frame,
            area,
            queue: self.queue,
        }
    }

    /// Queue a modal to be drawn after the main render tree is processed.
    ///
    /// The closure receives a mutable reference to the frame and the root area (full screen).
    /// Modals are drawn in the order they are queued.
    pub fn queue_modal<F>(&self, f: F)
    where
        F: FnOnce(&mut Frame<'b>, Rect) + 'b,
    {
        self.queue.modals.borrow_mut().push(Box::new(f));
    }

    /// Flush all queued modals, drawing them in order.
    ///
    /// This should be called once at the root level after all non-modal drawing is complete.
    pub fn flush_modals(&mut self) {
        let root_area = self.queue.root_area;
        let mut modals = self.queue.modals.borrow_mut();
        for modal in modals.drain(..) {
            modal(self.frame, root_area);
        }
    }
}
