//! Full stops and priority at mapped signs. California Driver's Handbook,
//! section 7; California Vehicle Code §§ 21800, 21801, 21802, and 22450.

use super::*;

/// Numerical tolerance at the surveyed station's millimetre precision.
const STOP_POSITION_EPS_M: f64 = 0.001;

pub(super) fn stop_station(net: &Network, car: &Car) -> Option<f64> {
    let lane = &net.lanes[car.lane as usize];
    let ControlKind::MappedStop { stop_offset_m, .. } = lane.control else { return None };
    let entry = lane.links.get(car.next as usize).and_then(|l| l.plan.as_ref()).map_or(0.0, |p| p.entry_setback);
    Some((lane.length - (stop_offset_m + car.len / 2.0).max(entry)).max(0.0))
}

/// Time to clear a movement at the fleet's acceleration and speed caps.
fn travel_time(distance: f64, speed: f64, cap: f64) -> f64 {
    let accel_time = ((cap - speed) / ACCEL).max(0.0);
    let ramp = speed * accel_time + ACCEL * accel_time * accel_time / 2.0;
    if distance <= ramp {
        ((speed * speed + 2.0 * ACCEL * distance).sqrt() - speed) / ACCEL
    } else {
        accel_time + (distance - ramp) / speed.max(cap).max(f64::EPSILON)
    }
}

/// True when the second vehicle is to the right of the first, or is the
/// opposing through movement at a tie. IDs give a stable order to the
/// remaining tied conflicting movements on the same or opposing sides.
fn yields_at_tie(net: &Network, a: &Car, b: &Car) -> bool {
    let direction = |car: &Car| {
        let lane = &net.lanes[car.lane as usize];
        let [u, v] = [lane.pts[lane.pts.len() - 2], lane.pts[lane.pts.len() - 1]];
        let direction = [v[0] - u[0], v[1] - u[1]];
        let turn = lane.links.get(car.next as usize).map(|l| l.movement);
        (direction, turn)
    };
    let ((a_dir, a_turn), (b_dir, b_turn)) = (direction(a), direction(b));
    let side = a_dir[0] * b_dir[1] - a_dir[1] * b_dir[0];
    let dot = a_dir[0] * b_dir[0] + a_dir[1] * b_dir[1];
    let steering_left = |m| matches!(m, Some(Movement::Left | Movement::UTurn));
    if side.abs() > dot.abs() * net.timing.axis_tolerance_rad.tan() {
        side < 0.0
    } else if dot < 0.0 && steering_left(a_turn) != steering_left(b_turn) {
        steering_left(a_turn)
    } else {
        b.id < a.id
    }
}

impl Fleet {
    pub(super) fn record_stops(&mut self, net: &Network, time: f64) {
        // Publish arrivals together so an exact tie never depends on act order.
        for car in &mut self.cars {
            if !car.cornering() && car.stop_arrival.is_none() && car.speed == 0.0
                && stop_station(net, car).is_some_and(|station| car.arc >= station - STOP_POSITION_EPS_M)
            {
                car.stop_arrival = Some(time);
                car.stop_released = true;
                self.stats.stop_sign_stops += 1;
            }
        }
    }

    pub(super) fn mapped_stop_clear(&self, ctx: &StepCtx, i: usize, movement: usize) -> bool {
        let net = ctx.net;
        let car = &self.cars[i];
        let Some(arrival) = car.stop_arrival else { return false };
        let lane = &net.lanes[car.lane as usize];
        let node = lane.end_node;
        let plan = lane.links[car.next as usize].plan.as_ref();
        let entry = plan.map_or(0.0, |p| p.entry_setback);
        let cap = plan.map_or(LOCAL_CORNER_SPEED_MPS, |p| LOCAL_CORNER_SPEED_MPS.min(p.speed_cap.max(1.2)));
        let distance = (lane.length - car.arc - entry).max(0.0)
            + plan.map_or(0.0, |p| p.arc_len) + car.len / 2.0 + CLAIM_RELEASE_MARGIN_M;
        if self.crossing_hold[i] <= distance || self.person_gap[i] <= distance {
            return false;
        }
        let clearance_time = travel_time(distance, car.speed, cap);

        let conflicts = |movement, other: &Car| {
            let approach = &net.lanes[other.lane as usize];
            approach.links.iter().enumerate().any(|(ix, link)| {
                (other.next == NO_LINK || other.next as usize == ix)
                    && link.legal && link.drivable
                    && net.movement_index(node, other.lane, ix as u16).is_some_and(|m| net.movements_conflict(node, movement, m))
            })
        };
        let mut ties = Vec::new();
        for &other_lane in &net.nodes[node as usize].incoming {
            let Some(j) = self.last_on_lane(other_lane) else { continue };
            let other = &self.cars[j];
            let approach = &net.lanes[other_lane as usize];
            if other_lane == car.lane || other.lane != other_lane || other.cornering()
                || !net.shares_surface(self.surface(i), self.surface(j))
            {
                continue;
            }
            match approach.control {
                ControlKind::MappedStop { .. } => {
                    if let Some(at) = other.stop_arrival {
                        if at < arrival && conflicts(movement, other) {
                            return false;
                        }
                        if at == arrival {
                            ties.push(j);
                        }
                    }
                }
                ControlKind::Stop { enforced: true, .. } if !other.stop_released => continue,
                ControlKind::Signal { junction, axis, .. }
                    if net.junctions[junction as usize].aspect_at(axis as usize, ctx.time) != GREEN => continue,
                _ if conflicts(movement, other) => {
                    // A priority approach can enter before this vehicle has
                    // cleared, using the actual separation and driving caps.
                    let entry = approach.links.get(other.next as usize).and_then(|link| link.plan.as_ref())
                        .map_or(0.0, |plan| plan.entry_setback);
                    let gap = (approach.length - other.arc - entry - other.len / 2.0).max(0.0);
                    if travel_time(gap, other.speed, other.max_speed) <= clearance_time {
                        return false;
                    }
                }
                _ => (),
            }
        }
        if !ties.iter().any(|&j| conflicts(movement, &self.cars[j]) && yields_at_tie(net, car, &self.cars[j])) {
            return true;
        }
        // If every tied approach owes priority to a conflicting driver,
        // give one driver the first movement so the group can clear.
        ties.push(i);
        let cycle = ties.iter().all(|&j| {
            let other = &self.cars[j];
            net.movement_index(node, other.lane, other.next).is_some_and(|m| {
                ties.iter().any(|&k| k != j && conflicts(m, &self.cars[k]) && yields_at_tie(net, other, &self.cars[k]))
            })
        });
        cycle && ties.iter().all(|&j| car.id <= self.cars[j].id)
    }
}
