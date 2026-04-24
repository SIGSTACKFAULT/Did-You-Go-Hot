use crate::{
    chart_gen::{
        ConnectionPass, Destination, NodeConnectionData, NodeData, PassDecision, RollingChart,
    },
    roll_calc::RollPlan,
};

fn plan_to_data(plan: &RollPlan) -> NodeData {
    NodeData {
        rollout_probability: plan.qualities.roll_out_probability,
    }
}

fn plan_id(plan: &RollPlan) -> usize {
    plan as *const RollPlan as usize
}

pub fn generate_roll_chart(plan: &RollPlan) -> RollingChart {
    let from_id = plan_id(plan);
    let mut chart = RollingChart::new(from_id, plan_to_data(plan));

    generate_roll_chart_rec(plan, &mut chart, from_id);

    chart.compress();
    chart
}

pub fn generate_roll_chart_rec(plan: &RollPlan, chart: &mut RollingChart, from_id: usize) {
    if plan.decision.can_close {
        chart
            .add_edge(from_id, Destination::Closed, PassDecision::Closed)
            .unwrap();
    }
    for (step, decision) in [
        (plan.decision.crit, PassDecision::Crit),
        (plan.decision.shrink, PassDecision::Shrink),
        (plan.decision.full, PassDecision::Full),
    ] {
        if let Some(step) = step {
            let next_id = plan_id(step.next_plan);
            chart.add_node(next_id, plan_to_data(step.next_plan));
            chart
                .add_edge(
                    from_id,
                    Destination::Node(NodeConnectionData {
                        to: next_id,
                        pass: ConnectionPass {
                            ship: step.ship,
                            state: step.ship_state,
                            direction: step.direction,
                        },
                    }),
                    decision,
                )
                .unwrap();
            generate_roll_chart_rec(step.next_plan, chart, next_id);
        }
    }
}
