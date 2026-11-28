use eo_sim::Region;
use eo_sim::network::TileKey;
use eo_sim::stats::Sample;
use serde::Serialize;

/// A fixed-focus window. Counts are observations, so one persistent overlap
/// can contribute to several samples.
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WindowAudit {
    pub lanes: usize,
    pub signal_controllers: usize,
    pub mapped_stop_lanes: usize,
    pub mapped_stop_ids: Vec<u64>,
    pub stop_sign_stops: u64,
    pub samples: usize,
    pub max_cars: usize,
    pub max_people: usize,
    pub person_samples: usize,
    pub crossing_person_samples: usize,
    pub crossings_started: u64,
    pub crossings_completed: u64,
    pub frames_with_vehicle_overlap: usize,
    pub frames_with_person_in_car: usize,
    pub frames_off_lane_center: usize,
    pub same_id_jumps: usize,
    pub surface_height_jumps: usize,
    pub biggest_surface_step_m: f64,
    pub red_entries: u64,
    pub walk_violations: u64,
    pub replacements: u64,
    pub first_issue_step: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_audit: Option<super::vehicle_scale::Summary>,
}

impl WindowAudit {
    pub fn observe(&mut self, r: &Region, sample: &Sample, step: usize, jumps: usize, surface_jumps: usize, biggest_surface_step: f64) {
        self.samples += 1;
        self.max_cars = self.max_cars.max(sample.cars);
        self.max_people = self.max_people.max(sample.walkers);
        self.person_samples += sample.walkers;
        self.crossing_person_samples += sample.crossing;
        self.crossings_started = r.crowd.stats.crossings_started;
        self.crossings_completed = r.crowd.stats.crossings_completed;
        self.frames_with_vehicle_overlap += usize::from(sample.overlaps != 0);
        self.frames_with_person_in_car += usize::from(sample.person_in_car != 0);
        self.frames_off_lane_center += usize::from(sample.off_lane != 0);
        self.same_id_jumps = jumps;
        self.surface_height_jumps = surface_jumps;
        self.biggest_surface_step_m = biggest_surface_step;
        self.red_entries = r.fleet.stats.red_entries;
        self.stop_sign_stops = r.fleet.stats.stop_sign_stops;
        self.walk_violations = r.crowd.stats.starts_outside_walk;
        self.replacements = r.stats.car_recycles;
        if sample.overlaps != 0
            || sample.person_in_car != 0
            || jumps != 0
            || surface_jumps != 0
            || self.red_entries != 0
            || self.walk_violations != 0
        {
            self.first_issue_step.get_or_insert(step);
        }
    }
}

#[derive(Serialize)]
pub(crate) struct TileAudit {
    pub map: String,
    #[serde(flatten)]
    pub observations: WindowAudit,
}

impl TileAudit {
    pub fn new(tile: TileKey) -> Self {
        Self {
            map: format!("tile:{},{}", tile.x, tile.z),
            observations: WindowAudit::default(),
        }
    }
}
