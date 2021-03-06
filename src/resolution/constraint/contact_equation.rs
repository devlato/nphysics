use std::num::Bounded;
use std::cell::Ref;
use nalgebra::na;
use ncollide::contact::Contact;
use ncollide::volumetric::InertiaTensor;
use ncollide::math::{N, LV, AV};
use resolution::constraint::velocity_constraint::VelocityConstraint;
use object::RigidBody;
use utils::ref_to::RefTo;

pub enum CorrectionMode {
    Velocity(N),
    VelocityAndPosition(N, N, N),
    VelocityAndPositionThresold(N, N, N)
}

impl CorrectionMode {
    #[inline]
    pub fn vel_corr_factor(&self) -> N {
        match *self {
            Velocity(ref v)                          => v.clone(),
            VelocityAndPosition(ref v, _, _)         => v.clone(),
            VelocityAndPositionThresold(ref v, _, _) => v.clone()
        }
    }

    #[inline]
    pub fn pos_corr_factor(&self) -> N {
        match *self {
            VelocityAndPosition(_, ref p, _)         => p.clone(),
            VelocityAndPositionThresold(_, ref p, _) => p.clone(),
            Velocity(_)                              => na::zero()
        }
    }

    #[inline]
    pub fn min_depth_for_pos_corr(&self) -> N {
        match *self {
            VelocityAndPosition(_, _, ref t)         => t.clone(),
            VelocityAndPositionThresold(_, _, ref t) => t.clone(),
            Velocity(_)                              => Bounded::max_value()
        }
    }

    #[inline]
    pub fn max_depth_for_vel_corr(&self) -> N {
        match *self {
            VelocityAndPosition(_, _, _)             => Bounded::max_value(),
            VelocityAndPositionThresold(_, _, ref t) => t.clone(),
            Velocity(_)                              => Bounded::max_value()
        }
    }
}

pub struct CorrectionParameters {
    corr_mode:       CorrectionMode,
    joint_corr:      N,
    rest_eps:        N
}

pub fn reinit_to_first_order_equation(dt:         N,
                                      coll:       &Contact,
                                      constraint: &mut VelocityConstraint,
                                      correction: &CorrectionParameters) {
    /*
     * Fill b
     */
    if coll.depth >= correction.corr_mode.min_depth_for_pos_corr() {
        constraint.objective = correction.corr_mode.pos_corr_factor() * coll.depth.max(&na::zero()) / dt;
    }
    else {
        constraint.objective = na::zero();
    }

    /*
     * Reset forces
     */
    constraint.impulse = na::zero();
}

pub fn fill_second_order_equation(dt:           N,
                                  coll:         &Contact,
                                  rb1:          &Ref<RigidBody>,
                                  rb2:          &Ref<RigidBody>,
                                  rconstraint:  &mut VelocityConstraint,
                                  idr:          uint,
                                  fconstraints: &mut [VelocityConstraint],
                                  idf:          uint,
                                  cache:        &[N],
                                  correction:   &CorrectionParameters) {
    let restitution = rb1.get().restitution() * rb2.get().restitution();

    let center = (coll.world1 + coll.world2) * na::cast::<f32, N>(0.5);

    fill_velocity_constraint(dt.clone(),
                             coll.normal.clone(),
                             center.clone(),
                             restitution,
                             coll.depth.clone(),
                             cache[0].clone(), // coll.impulses[0].clone(),
                             na::zero(),
                             Bounded::max_value(),
                             rb1,
                             rb2,
                             rconstraint,
                             correction);


    let friction  = rb1.get().friction() * rb2.get().friction();
    // To bound the friction we use the last frame normal impulse.
    // That means we have to make a special case for the first time the contact appears.
    // In that case, we estimate the impulse by the derired normal correction.

    let mut i = 0;

    na::orthonormal_subspace_basis(&coll.normal, |friction_axis| {
        let constraint = &mut fconstraints[idf + i];

        fill_velocity_constraint(dt.clone(),
                                 friction_axis,
                                 center.clone(),
                                 na::zero(),
                                 na::zero(),
                                 cache[i + 1].clone(), // coll.impulses[i].clone(),
                                 na::zero(), // dont setup the limit now
                                 na::zero(), // dont setup the limit now
                                 rb1,
                                 rb2,
                                 constraint,
                                 correction);

        constraint.friction_coeff    = friction.clone();
        constraint.friction_limit_id = idr;
        i = i + 1;

        true
    })
}

pub fn fill_constraint_geometry<R: RefTo<RigidBody>>(
                                normal:     LV,
                                rot_axis1:  AV,
                                rot_axis2:  AV,
                                rb1:        &Option<R>,
                                rb2:        &Option<R>,
                                constraint: &mut VelocityConstraint) {
    constraint.normal             = normal;
    constraint.inv_projected_mass = na::zero();

    match *rb1 {
        Some(ref b) => {
            let rb = b.get();
            // rotation axis
            constraint.weighted_normal1   = constraint.normal * rb.inv_mass();
            constraint.rot_axis1          = rot_axis1;

            constraint.weighted_rot_axis1 = rb.inv_inertia().apply(&constraint.rot_axis1);

            constraint.inv_projected_mass = constraint.inv_projected_mass +
                na::dot(&constraint.normal, &constraint.weighted_normal1) +
                na::dot(&constraint.rot_axis1, &constraint.weighted_rot_axis1);
        },
        None => { }
    }

    match *rb2 {
        Some(ref b) => {
            let rb = b.get();
            // rotation axis
            constraint.weighted_normal2   = constraint.normal * rb.inv_mass();
            constraint.rot_axis2          = rot_axis2;

            constraint.weighted_rot_axis2 = rb.inv_inertia().apply(&constraint.rot_axis2);

            constraint.inv_projected_mass = constraint.inv_projected_mass +
                na::dot(&constraint.normal, &constraint.weighted_normal2) +
                na::dot(&constraint.rot_axis2, &constraint.weighted_rot_axis2);
        },
        None => { }
    }

    let _1: N = na::one();
    constraint.inv_projected_mass = _1 / constraint.inv_projected_mass;
}

fn fill_velocity_constraint(dt:              N,
                            normal:          LV,
                            center:          LV,
                            restitution:     N,
                            depth:           N,
                            initial_impulse: N,
                            lobound:         N,
                            hibound:         N,
                            rb1:             &Ref<RigidBody>,
                            rb2:             &Ref<RigidBody>,
                            constraint:      &mut VelocityConstraint,
                            correction:      &CorrectionParameters) {
    let rot_axis1 = na::cross(&(center - *rb1.get().center_of_mass()), &-normal);
    let rot_axis2 = na::cross(&(center - *rb2.get().center_of_mass()), &normal);

    let opt_rb1 = if rb1.get().can_move() { Some(rb1.get()) } else { None };
    let opt_rb2 = if rb2.get().can_move() { Some(rb2.get()) } else { None };
    fill_constraint_geometry(normal, rot_axis1, rot_axis2, &opt_rb1, &opt_rb2, constraint);

    /*
     * Fill indice
     */
    constraint.id1 = rb1.get().index();
    constraint.id2 = rb2.get().index();

    /*
     * correction amount
     */
    constraint.objective = relative_velocity(
        &opt_rb1,
        &opt_rb2,
        &constraint.normal,
        &constraint.rot_axis1,
        &constraint.rot_axis2,
        &dt);

    if constraint.objective < -correction.rest_eps {
        constraint.objective = constraint.objective + restitution * constraint.objective
    }

    constraint.objective = -constraint.objective;

    if depth < na::zero() {
        constraint.objective = constraint.objective + depth / dt
    }
    else if depth < correction.corr_mode.max_depth_for_vel_corr() {
        constraint.objective = constraint.objective + depth * correction.corr_mode.vel_corr_factor() / dt
    }

    // for warm-starting
    constraint.impulse = if depth < na::zero() { na::zero() } else { initial_impulse };

    /*
     * constraint bounds
     */
    constraint.lobound = lobound;
    constraint.hibound = hibound;
}

pub fn relative_velocity<R: RefTo<RigidBody>>(
                         rb1:       &Option<R>,
                         rb2:       &Option<R>,
                         normal:    &LV,
                         rot_axis1: &AV,
                         rot_axis2: &AV,
                         dt:        &N)
                         -> N {
    let mut dvel: N = na::zero();

    match *rb1 {
        Some(ref b) => {
            let rb = b.get();
            dvel = dvel - na::dot(&(rb.lin_vel() + rb.lin_acc() * *dt), normal)
                        + na::dot(&(rb.ang_vel() + rb.ang_acc() * *dt), rot_axis1);
        },
        None => { }
    }

    match *rb2 {
        Some(ref b) => {
            let rb = b.get();
            dvel = dvel + na::dot(&(rb.lin_vel() + rb.lin_acc() * *dt), normal)
                        + na::dot(&(rb.ang_vel() + rb.ang_acc() * *dt), rot_axis2);
        },
        None => { }
    }

    dvel
}
