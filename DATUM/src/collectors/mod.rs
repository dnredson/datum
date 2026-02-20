pub trait Collector {
    fn name(&self) -> &'static str;
    fn collect(&self, ctx: &CycleContext) -> CollectorOutput;
}

#[derive(Default)]
pub struct CollectorOutput {
    pub entities: Vec<Entity>,
    pub edges: Vec<Edge>,
    pub events: Vec<Event>,
}
