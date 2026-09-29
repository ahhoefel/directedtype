use std::ops::{Deref, DerefMut};

use crate::compiler::layout::ResolvedLayout;
use crate::dom::error::DomError;
use crate::dom::Dom;

/// An active transaction context over the Component DOM.
///
/// Implements `Deref` and `DerefMut` targeting `Dom`, exposing all mutation
/// and reflection methods. Changes made within the transaction modify the
/// Component DOM immediately in memory, and commit layout changes on completion.
pub struct Transaction<'a> {
    pub(crate) dom: &'a mut Dom,
    pub(crate) committed: bool,
}

impl<'a> Transaction<'a> {
    pub(crate) fn new(dom: &'a mut Dom) -> Self {
        Self {
            dom,
            committed: false,
        }
    }

    /// Explicitly commits all staged changes in this transaction immediately.
    pub fn commit(mut self) -> Result<&'a ResolvedLayout, DomError> {
        self.committed = true;
        self.dom.commit()
    }
}

impl<'a> Deref for Transaction<'a> {
    type Target = Dom;

    fn deref(&self) -> &Self::Target {
        self.dom
    }
}

impl<'a> DerefMut for Transaction<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.dom
    }
}
