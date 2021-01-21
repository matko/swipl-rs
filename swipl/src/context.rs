use super::term::*;
use super::engine::*;

use swipl_sys::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Context<'a,T:ContextType> {
    parent: Option<&'a dyn ContextParent>,
    context: T,
    engine: PL_engine_t,
    activated: AtomicBool
}

impl<'a,T:ContextType> Context<'a,T> {
    fn assert_activated(&self) {
        if !self.activated.load(Ordering::Relaxed) {
            panic!("cannot acquire term refs from inactive context");
        }
    }
    pub fn new_term_ref(&self) -> Term {
        self.assert_activated();
        unsafe {
            let term = PL_new_term_ref();
            Term::new(term, self)
        }
    }

    pub unsafe fn wrap_term_ref(&self, term: term_t) -> Term {
        self.assert_activated();
        Term::new(term, self)
    }

    pub fn open_frame(&self) -> Context<Frame> {
        self.assert_activated();
        let fid = unsafe {PL_open_foreign_frame()};

        let frame = Frame {
            fid,
            state: FrameState::Active
        };

        self.activated.store(false, Ordering::Relaxed);
        Context {
            parent: Some(self),
            context: frame,
            engine: self.engine,
            activated: AtomicBool::new(true)
        }
    }
}

trait ContextParent {
    fn reactivate(&self);
}

impl<'a,T:ContextType> ContextParent for Context<'a,T> {
    fn reactivate(&self) {
        if self.activated.compare_and_swap(false, true, Ordering::Acquire) {
            panic!("context already active");
        }
    }
}

impl<'a,T:ContextType> TermOrigin for Context<'a,T> {
    fn is_engine_active(&self) -> bool {
        is_engine_active(self.engine)
    }

    fn origin_engine_ptr(&self) -> PL_engine_t {
        self.engine
    }
}

impl<'a,T:ContextType> Drop for Context<'a,T> {
    fn drop(&mut self) {
        if let Some(parent) = self.parent {
            parent.reactivate();
        }
    }
}

pub trait ContextType {}

pub struct ActivatedEngine<'a> {
    _activation: EngineActivation<'a>
}

impl<'a> Into<Context<'a,ActivatedEngine<'a>>> for EngineActivation<'a> {
    fn into(self) -> Context<'a, ActivatedEngine<'a>> {
        let engine = self.engine_ptr();
        let context = ActivatedEngine {
            _activation: self
        };

        Context {
            parent: None,
            context,
            engine,
            activated: AtomicBool::new(true)
        }
    }
}

impl<'a> ContextType for ActivatedEngine<'a> {}

pub struct UnmanagedContext;
impl ContextType for UnmanagedContext {}

// This is unsafe to call if we are not in a swipl environment, or if some other context is active. Furthermore, the lifetime will most definitely be wrong. This should be used by code that doesn't promiscuously spread this context. all further accesses should be through borrows.
pub unsafe fn unmanaged_engine_context() -> Context<'static, UnmanagedContext> {
    let current = current_engine_ptr();

    if current.is_null() {
        panic!("tried to create an unmanaged engine context, but no engine is active");
    }
    
    Context {
        parent: None,
        context: UnmanagedContext,
        engine: current,
        activated: AtomicBool::new(true)
    }
}

enum FrameState {
    Active,
    Discarded
}

pub struct Frame {
    fid: PL_fid_t,
    state: FrameState
}

impl ContextType for Frame {}

impl Drop for Frame {
    fn drop(&mut self) {
        match &self.state {
            FrameState::Active => 
            // unsafe justification: all instantiations of Frame happen in
            // this module.  This module only instantiates the frame as
            // part of the context mechanism. No 'free' Frames are ever
            // returned.  This mechanism ensures that the frame is only
            // closed if there's no inner frame still remaining. It'll
            // also ensure that the engine of the frame is active while
            // dropping.
                unsafe { PL_close_foreign_frame(self.fid) },
            _ => {}
        }
    }
}

impl<'a> Context<'a, Frame> {
    pub fn close_frame(self) {
        // would happen automatically but might as well be explicit
        std::mem::drop(self)
    }

    pub fn discard_frame(mut self) {
        self.context.state = FrameState::Discarded;
        // unsafe justification: reasons for safety are the same as in a normal drop. Also, sicne we just set framestate to discarded, the drop won't try to subsequently close this same frame.
        unsafe { PL_discard_foreign_frame(self.context.fid) };
    }

    pub fn rewind_frame(&self) {
        self.assert_activated();
        // unsafe justification: We just checked that this frame right here is currently the active context. Therefore it can be rewinded.
        unsafe { PL_rewind_foreign_frame(self.context.fid) };
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn get_term_ref_on_fresh_engine() {
        initialize_swipl_noengine();
        let engine = Engine::new();
        let activation = engine.activate();
        let context: Context<_> = activation.into();

        let _term = context.new_term_ref();
    }

    #[test]
    fn get_term_ref_on_frame() {
        initialize_swipl_noengine();
        let engine = Engine::new();
        let activation = engine.activate();
        let context1: Context<_> = activation.into();
        let _term1 = context1.new_term_ref();

        let context2 = context1.open_frame();
        let _term2 = context2.new_term_ref();
        std::mem::drop(context2);
        let _term3 = context1.new_term_ref();
    }

    #[test]
    #[should_panic]
    fn get_term_ref_from_inactive_context_panics() {
        initialize_swipl_noengine();
        let engine = Engine::new();
        let activation = engine.activate();
        let context1: Context<_> = activation.into();
        let _context2 = context1.open_frame();

        let _term = context1.new_term_ref();
    }
}