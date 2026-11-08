use std::any::Any;

/// An opaque capability a tool source attaches to one sampling step's plan.
///
/// The kernel never inspects it: it keeps every lease the plan's sources
/// returned alive for as long as the plan, and hands them to each call as
/// [`super::ToolInvocation::plan_leases`]. A source's own tools find theirs by
/// downcasting, so a refresh between sampling and the call cannot replace the
/// binding the model was shown.
///
/// Every `'static` type is a lease, `Arc<dyn ToolPlanLease>` included, so
/// downcast through the trait object (`lease.as_ref().as_any()`) or
/// [`super::ToolInvocation::plan_lease`], never through the `Arc`.
pub trait ToolPlanLease: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Send + Sync> ToolPlanLease for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
