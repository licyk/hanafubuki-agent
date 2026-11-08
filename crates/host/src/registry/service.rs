use std::sync::Arc;

use kardia_core::{Disposer, EffectHandle, Service, Svc};

use super::{RegistrationError, Registry};

/// A kardia service that exposes one [`Registry`].
pub trait RegistryService: Service {
    type Item: Send + Sync + 'static;

    fn registry(&self) -> &Registry<Self::Item>;

    /// Decides whether `candidate` may join `existing`. Single-provider slots
    /// reject a second entry; keyed registries reject duplicate identities.
    fn admit(existing: &[&Self::Item], candidate: &Self::Item) -> Result<(), RegistrationError> {
        let _ = (existing, candidate);
        Ok(())
    }
}

/// Registration through an injected registry service.
///
/// The registration is an effect of the *calling* plugin (`Svc::caller`): it
/// is reverted when that plugin stops, after calls holding a lease on the
/// entry have drained. Plugins never keep a handle to undo it themselves.
pub trait Register<S: RegistryService> {
    fn register(&self, item: S::Item) -> kardia_core::Result<EffectHandle>;
}

impl<S: RegistryService> Register<S> for Svc<S> {
    fn register(&self, item: S::Item) -> kardia_core::Result<EffectHandle> {
        let service: Arc<S> = self.arc().clone();
        let caller = self.caller();
        let owner = caller.fiber().name();
        caller.effect_sync(format!("{}.register()", S::NAME), move |scope| {
            let entry = service
                .registry()
                .insert(owner, item, S::admit)
                .map_err(kardia_core::Error::new)?;
            scope.defer(Disposer::future(move || async move {
                service.registry().retire(&entry).await;
            }));
            Ok(())
        })
    }
}
