use std::{
    collections::BTreeMap,
    fmt::Display,
    hash::Hash,
    ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub},
};

use itertools::Itertools;
use num_traits::Zero;
use powdr_number::{log2_exact, FieldElement, LargeInt};

use crate::witgen::{
    jit::effect::{Assertion, BitDecomposition, BitDecompositionComponent, Effect},
    range_constraints::RangeConstraint,
};

use super::{symbolic_expression::SymbolicExpression, variable_update::VariableUpdate};

#[derive(Default)]
pub struct ProcessResult<T: FieldElement, V> {
    pub effects: Vec<Effect<T, V>>,
    pub complete: bool,
}

impl<T: FieldElement, V> ProcessResult<T, V> {
    pub fn empty() -> Self {
        Self {
            effects: vec![],
            complete: false,
        }
    }
    pub fn complete(effects: Vec<Effect<T, V>>) -> Self {
        Self {
            effects,
            complete: true,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    /// The range constraints of the parts do not cover the full constant sum.
    ConflictingRangeConstraints,
    /// An equality constraint evaluates to a known-nonzero value.
    ConstraintUnsatisfiable,
}

/// A symbolic expression in unknown variables of type `V` and (symbolically)
/// known terms, representing a sum of (super-)quadratic, linear and constant parts.
/// The quadratic terms are of the form `X * Y`, where `X` and `Y` are
/// `QuadraticSymbolicExpression`s that have at least one unknown.
/// The linear terms are of the form `a * X`, where `a` is a (symbolically) known
/// value and `X` is an unknown variable.
/// The constant term is a (symbolically) known value.
///
/// It also provides ways to quickly update the expression when the value of
/// an unknown variable gets known and provides functions to solve
/// (some kinds of) equations.
#[derive(Debug, Clone)]
pub struct QuadraticSymbolicExpression<T: FieldElement, V> {
    /// Quadratic terms of the form `a * X * Y`, where `a` is a (symbolically)
    /// known value and `X` and `Y` are quadratic symbolic expressions that
    /// have at least one unknown.
    quadratic: Vec<(Self, Self)>,
    /// Linear terms of the form `a * X`, where `a` is a (symbolically) known
    /// value and `X` is an unknown variable.
    linear: BTreeMap<V, SymbolicExpression<T, V>>,
    /// Constant term, a (symbolically) known value.
    constant: SymbolicExpression<T, V>,
}

impl<T: FieldElement, V> From<SymbolicExpression<T, V>> for QuadraticSymbolicExpression<T, V> {
    fn from(k: SymbolicExpression<T, V>) -> Self {
        Self {
            quadratic: Default::default(),
            linear: Default::default(),
            constant: k,
        }
    }
}

impl<T: FieldElement, V> From<T> for QuadraticSymbolicExpression<T, V> {
    fn from(k: T) -> Self {
        SymbolicExpression::from(k).into()
    }
}

impl<T: FieldElement, V: Ord + Clone + Hash + Eq> QuadraticSymbolicExpression<T, V> {
    pub fn from_known_symbol(symbol: V, rc: RangeConstraint<T>) -> Self {
        SymbolicExpression::from_symbol(symbol, rc).into()
    }

    pub fn from_unknown_variable(var: V) -> Self {
        Self {
            quadratic: Default::default(),
            linear: [(var.clone(), T::from(1).into())].into_iter().collect(),
            constant: T::from(0).into(),
        }
    }

    /// If this expression does not contain unknown variables, returns the symbolic expression.
    pub fn try_to_known(&self) -> Option<&SymbolicExpression<T, V>> {
        if self.quadratic.is_empty() && self.linear.is_empty() {
            Some(&self.constant)
        } else {
            None
        }
    }

    /// Returns true if this expression contains at least one quadratic term.
    pub fn is_quadratic(&self) -> bool {
        !self.quadratic.is_empty()
    }

    pub fn apply_update(&mut self, var_update: &VariableUpdate<T, V>) {
        let VariableUpdate {
            variable,
            known,
            range_constraint,
        } = var_update;
        self.constant.apply_update(var_update);
        if self.linear.contains_key(variable) {
            // If the variable is a key in `linear`, it must be unknown
            // and thus can only occur there. Otherwise, it can be in
            // any symbolic expression.
            if *known {
                let coeff = self.linear.remove(variable).unwrap();
                let expr =
                    SymbolicExpression::from_symbol(variable.clone(), range_constraint.clone());
                self.constant += expr * coeff;
            }
        } else {
            for coeff in self.linear.values_mut() {
                coeff.apply_update(var_update);
            }
            self.linear.retain(|_, f| !f.is_known_zero());
        }

        // TODO can we do that without moving everything?
        // In the end, the order does not matter much.

        let mut to_add = QuadraticSymbolicExpression::from(T::zero());
        self.quadratic.retain_mut(|(l, r)| {
            l.apply_update(var_update);
            r.apply_update(var_update);
            match (l.try_to_known(), r.try_to_known()) {
                (Some(l), Some(r)) => {
                    to_add += (l * r).into();
                    false
                }
                (Some(l), None) => {
                    to_add += r.clone() * l;
                    false
                }
                (None, Some(r)) => {
                    to_add += l.clone() * r;
                    false
                }
                _ => true,
            }
        });
        if to_add.try_to_known().map(|ta| ta.is_known_zero()) != Some(true) {
            *self += to_add;
        }
    }

    /// Returns the set of referenced variables, both know and unknown.
    pub fn referenced_variables(&self) -> Box<dyn Iterator<Item = &V> + '_> {
        let quadr = self
            .quadratic
            .iter()
            .flat_map(|(a, b)| a.referenced_variables().chain(b.referenced_variables()));

        let linear = self
            .linear
            .iter()
            .flat_map(|(var, coeff)| std::iter::once(var).chain(coeff.referenced_symbols()));
        let constant = self.constant.referenced_symbols();
        Box::new(quadr.chain(linear).chain(constant))
    }

    /// Returns the set of referenced unknown variables.
    pub fn referenced_unknown_variables(&self) -> Box<dyn Iterator<Item = &V> + '_> {
        let quadratic = self.quadratic.iter().flat_map(|(a, b)| {
            a.referenced_unknown_variables()
                .chain(b.referenced_unknown_variables())
        });
        Box::new(quadratic.chain(self.linear.keys()))
    }
}

pub trait RangeConstraintProvider<T: FieldElement, V> {
    fn get(&self, var: &V) -> RangeConstraint<T>;
}

impl<T: FieldElement, V: Ord + Clone + Hash + Eq + Display> QuadraticSymbolicExpression<T, V> {
    /// Solves the equation `self = 0` and returns how to compute the solution.
    /// The solution can contain assignments to multiple variables.
    /// If no way to solve the equation (and no way to derive new range
    /// constraints) has been found, but it still contains
    /// unknown variables, returns an empty, incomplete result.
    /// If the equation is known to be unsolvable, returns an error.
    pub fn solve(
        &self,
        range_constraints: &impl RangeConstraintProvider<T, V>,
    ) -> Result<ProcessResult<T, V>, Error> {
        Ok(if self.is_quadratic() {
            ProcessResult::empty()
        } else if let Some(k) = self.try_to_known() {
            if k.is_known_nonzero() {
                return Err(Error::ConstraintUnsatisfiable);
            } else {
                // TODO we could still process more information
                // and reach "unsatisfiable" here.
                ProcessResult::complete(vec![])
            }
        } else {
            self.solve_affine(range_constraints)?
        })
    }

    fn solve_affine(
        &self,
        range_constraints: &impl RangeConstraintProvider<T, V>,
    ) -> Result<ProcessResult<T, V>, Error> {
        Ok(if self.linear.len() == 1 {
            let (var, coeff) = self.linear.iter().next().unwrap();
            // Solve "coeff * X + self.constant = 0" by division.
            assert!(
                !coeff.is_known_zero(),
                "Zero coefficient has not been removed: {self}"
            );
            if coeff.is_known_nonzero() {
                // In this case, we can always compute a solution.
                let value = self.constant.field_div(&-coeff);
                ProcessResult::complete(vec![Effect::Assignment(var.clone(), value)])
            } else if self.constant.is_known_nonzero() {
                // If the offset is not zero, then the coefficient must be non-zero,
                // otherwise the constraint is violated.
                let value = self.constant.field_div(&-coeff);
                ProcessResult::complete(vec![
                    Assertion::assert_is_nonzero(coeff.clone()),
                    Effect::Assignment(var.clone(), value),
                ])
            } else {
                // If this case, we could have an equation of the form
                // 0 * X = 0, which is valid and generates no information about X.
                ProcessResult::empty()
            }
        } else {
            // Solve expression of the form `a * X + b * Y + ... + self.constant = 0`
            let r = self.solve_bit_decomposition(range_constraints)?;

            if r.complete {
                r
            } else {
                let negated = -self;
                let effects = self
                    .transfer_constraints(range_constraints)
                    .into_iter()
                    .chain(negated.transfer_constraints(range_constraints))
                    .collect();
                ProcessResult {
                    effects,
                    complete: false,
                }
            }
        })
    }

    /// Tries to solve a bit-decomposition equation.
    fn solve_bit_decomposition(
        &self,
        range_constraints: &impl RangeConstraintProvider<T, V>,
    ) -> Result<ProcessResult<T, V>, Error> {
        assert!(!self.is_quadratic());
        // All the coefficients need to be known numbers and the
        // variables need to be range-constrained.
        let constrained_coefficients = self
            .linear
            .iter()
            .map(|(var, coeff)| {
                let coeff = coeff.try_to_number()?;
                let rc = range_constraints.get(var);
                let is_negative = !coeff.is_in_lower_half();
                let coeff_abs = if is_negative { -coeff } else { coeff };
                // We could work with non-powers of two, but it would require
                // division instead of shifts.
                let exponent = log2_exact(coeff_abs.to_arbitrary_integer())?;
                // We negate here because we are solving
                // c_1 * x_1 + c_2 * x_2 + ... + offset = 0,
                // instead of
                // c_1 * x_1 + c_2 * x_2 + ... = offset.
                Some((var.clone(), rc, !is_negative, coeff_abs, exponent))
            })
            .collect::<Option<Vec<_>>>();
        let Some(constrained_coefficients) = constrained_coefficients else {
            return Ok(ProcessResult::empty());
        };

        // If the offset is a known number, we gradually remove the
        // components from this number.
        let mut offset = self.constant.try_to_number();
        let mut concrete_assignments = vec![];

        // Check if they are mutually exclusive and compute assignments.
        let mut covered_bits: <T as FieldElement>::Integer = 0.into();
        let mut components = vec![];
        for (variable, constraint, is_negative, coeff_abs, exponent) in constrained_coefficients
            .into_iter()
            .sorted_by_key(|(_, _, _, _, exponent)| *exponent)
        {
            let bit_mask = *constraint.multiple(coeff_abs).mask();
            if !(bit_mask & covered_bits).is_zero() {
                // Overlapping range constraints.
                return Ok(ProcessResult::empty());
            } else {
                covered_bits |= bit_mask;
            }

            // If the offset is a known number, we create concrete assignments and modify the offset.
            // if it is not known, we return a BitDecomposition effect.
            if let Some(offset) = &mut offset {
                let mut component = if is_negative { -*offset } else { *offset }.to_integer();
                if component > (T::modulus() - 1.into()) >> 1 {
                    // Convert a signed finite field element into two's complement.
                    // a regular subtraction would underflow, so we do this.
                    // We add the difference between negative numbers in the field
                    // and negative numbers in two's complement.
                    component += T::Integer::MAX - T::modulus() + 1.into();
                };
                component &= bit_mask;
                concrete_assignments.push(Effect::Assignment(
                    variable.clone(),
                    T::from(component >> exponent).into(),
                ));
                if is_negative {
                    *offset += T::from(component);
                } else {
                    *offset -= T::from(component);
                }
            } else {
                components.push(BitDecompositionComponent {
                    variable,
                    is_negative,
                    exponent: exponent as u64,
                    bit_mask,
                });
            }
        }

        if covered_bits >= T::modulus() {
            return Ok(ProcessResult::empty());
        }

        if let Some(offset) = offset {
            if offset != 0.into() {
                return Err(Error::ConstraintUnsatisfiable);
            }
            assert_eq!(concrete_assignments.len(), self.linear.len());
            Ok(ProcessResult::complete(concrete_assignments))
        } else {
            Ok(ProcessResult::complete(vec![Effect::BitDecomposition(
                BitDecomposition {
                    value: self.constant.clone(),
                    components,
                },
            )]))
        }
    }

    fn transfer_constraints(
        &self,
        range_constraints: &impl RangeConstraintProvider<T, V>,
    ) -> Option<Effect<T, V>> {
        // We are looking for X = a * Y + b * Z + ... or -X = a * Y + b * Z + ...
        // where X is least constrained.

        assert!(!self.is_quadratic());

        let (solve_for, solve_for_coefficient) = self
            .linear
            .iter()
            .filter(|(_var, coeff)| coeff.is_known_one() || coeff.is_known_minus_one())
            .max_by_key(|(var, _c)| {
                // Sort so that we get the least constrained variable.
                range_constraints.get(var).range_width()
            })?;

        // This only works if the coefficients are all known.
        let summands = self
            .linear
            .iter()
            .filter(|(var, _)| *var != solve_for)
            .map(|(var, coeff)| {
                let coeff = coeff.try_to_number()?;
                Some(range_constraints.get(var).multiple(coeff))
            })
            .chain(std::iter::once(Some(self.constant.range_constraint())))
            .collect::<Option<Vec<_>>>()?;
        let constraint = summands.into_iter().reduce(|c1, c2| c1.combine_sum(&c2))?;
        let constraint = if solve_for_coefficient.is_known_one() {
            -constraint
        } else {
            constraint
        };
        Some(Effect::RangeConstraint(solve_for.clone(), constraint))
    }
}

impl<T: FieldElement, V: Clone + Ord + Hash + Eq> Add for QuadraticSymbolicExpression<T, V> {
    type Output = QuadraticSymbolicExpression<T, V>;

    fn add(mut self, rhs: Self) -> Self {
        self += rhs;
        self
    }
}

impl<T: FieldElement, V: Clone + Ord + Hash + Eq> Add for &QuadraticSymbolicExpression<T, V> {
    type Output = QuadraticSymbolicExpression<T, V>;

    fn add(self, rhs: Self) -> Self::Output {
        self.clone() + rhs.clone()
    }
}

impl<T: FieldElement, V: Clone + Ord + Hash + Eq> AddAssign<QuadraticSymbolicExpression<T, V>>
    for QuadraticSymbolicExpression<T, V>
{
    fn add_assign(&mut self, rhs: Self) {
        self.quadratic.extend(rhs.quadratic);
        for (var, coeff) in rhs.linear {
            self.linear
                .entry(var.clone())
                .and_modify(|f| *f += coeff.clone())
                .or_insert_with(|| coeff);
        }
        self.constant += rhs.constant.clone();
        self.linear.retain(|_, f| !f.is_known_zero());
    }
}

impl<T: FieldElement, V: Clone + Ord + Hash + Eq> Sub for &QuadraticSymbolicExpression<T, V> {
    type Output = QuadraticSymbolicExpression<T, V>;

    fn sub(self, rhs: Self) -> Self::Output {
        self + &-rhs
    }
}

impl<T: FieldElement, V: Clone + Ord + Hash + Eq> Sub for QuadraticSymbolicExpression<T, V> {
    type Output = QuadraticSymbolicExpression<T, V>;

    fn sub(self, rhs: Self) -> Self::Output {
        &self - &rhs
    }
}

impl<T: FieldElement, V: Clone + Ord> QuadraticSymbolicExpression<T, V> {
    fn negate(&mut self) {
        for (first, _) in &mut self.quadratic {
            first.negate()
        }
        for coeff in self.linear.values_mut() {
            *coeff = -coeff.clone();
        }
        self.constant = -self.constant.clone();
    }
}

impl<T: FieldElement, V: Clone + Ord> Neg for QuadraticSymbolicExpression<T, V> {
    type Output = QuadraticSymbolicExpression<T, V>;

    fn neg(mut self) -> Self {
        self.negate();
        self
    }
}

impl<T: FieldElement, V: Clone + Ord> Neg for &QuadraticSymbolicExpression<T, V> {
    type Output = QuadraticSymbolicExpression<T, V>;

    fn neg(self) -> Self::Output {
        -((*self).clone())
    }
}

/// Multiply by known symbolic expression.
impl<T: FieldElement, V: Clone + Ord + Hash + Eq> Mul<&SymbolicExpression<T, V>>
    for QuadraticSymbolicExpression<T, V>
{
    type Output = QuadraticSymbolicExpression<T, V>;

    fn mul(mut self, rhs: &SymbolicExpression<T, V>) -> Self {
        self *= rhs;
        self
    }
}

impl<T: FieldElement, V: Clone + Ord + Hash + Eq> Mul<SymbolicExpression<T, V>>
    for QuadraticSymbolicExpression<T, V>
{
    type Output = QuadraticSymbolicExpression<T, V>;

    fn mul(self, rhs: SymbolicExpression<T, V>) -> Self {
        self * &rhs
    }
}

impl<T: FieldElement, V: Clone + Ord + Hash + Eq> MulAssign<&SymbolicExpression<T, V>>
    for QuadraticSymbolicExpression<T, V>
{
    fn mul_assign(&mut self, rhs: &SymbolicExpression<T, V>) {
        if rhs.is_known_zero() {
            *self = T::zero().into();
        } else {
            for (first, _) in &mut self.quadratic {
                *first *= rhs;
            }
            for coeff in self.linear.values_mut() {
                *coeff *= rhs.clone();
            }
            self.constant *= rhs.clone();
        }
    }
}

impl<T: FieldElement, V: Clone + Ord + Hash + Eq> Mul for QuadraticSymbolicExpression<T, V> {
    type Output = QuadraticSymbolicExpression<T, V>;

    fn mul(self, rhs: QuadraticSymbolicExpression<T, V>) -> Self {
        if let Some(k) = rhs.try_to_known() {
            self * k
        } else if let Some(k) = self.try_to_known() {
            rhs * k
        } else {
            Self {
                quadratic: vec![(self, rhs)],
                linear: Default::default(),
                constant: T::from(0).into(),
            }
        }
    }
}

impl<T: FieldElement, V: Clone + Ord + Display> Display for QuadraticSymbolicExpression<T, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let formatted = self
            .quadratic
            .iter()
            .map(|(a, b)| format!("({a}) * ({b})"))
            .chain(
                self.linear
                    .iter()
                    .map(|(var, coeff)| match coeff.try_to_number() {
                        Some(k) if k == 1.into() => format!("{var}"),
                        Some(k) if k == (-1).into() => format!("-{var}"),
                        _ => format!("{coeff} * {var}"),
                    }),
            )
            .chain(match self.constant.try_to_number() {
                Some(k) if k == T::zero() => None,
                _ => Some(format!("{}", self.constant)),
            })
            .format(" + ")
            .to_string();
        write!(
            f,
            "{}",
            if formatted.is_empty() {
                "0".to_string()
            } else {
                formatted
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::witgen::range_constraints::RangeConstraint;
    use powdr_number::GoldilocksField;

    use pretty_assertions::assert_eq;

    type Qse = QuadraticSymbolicExpression<GoldilocksField, &'static str>;

    #[test]
    fn test_mul() {
        let x = Qse::from_unknown_variable("X");
        let y = Qse::from_unknown_variable("Y");
        let a = Qse::from_known_symbol("A", RangeConstraint::default());
        let t = x * y + a;
        assert_eq!(t.to_string(), "(X) * (Y) + A");
    }

    #[test]
    fn test_add() {
        let x = Qse::from_unknown_variable("X");
        let y = Qse::from_unknown_variable("Y");
        let a = Qse::from_unknown_variable("A");
        let b = Qse::from_known_symbol("B", RangeConstraint::default());
        let t: Qse = x * y - a + b;
        assert_eq!(t.to_string(), "(X) * (Y) + -A + B");
        assert_eq!(
            (t.clone() + t).to_string(),
            "(X) * (Y) + (X) * (Y) + -2 * A + (B + B)"
        );
    }

    #[test]
    fn test_mul_by_known() {
        let x = Qse::from_unknown_variable("X");
        let y = Qse::from_unknown_variable("Y");
        let a = Qse::from_known_symbol("A", RangeConstraint::default());
        let b = Qse::from_known_symbol("B", RangeConstraint::default());
        let t: Qse = (x * y + a) * b;
        assert_eq!(t.to_string(), "(B * X) * (Y) + (A * B)");
    }

    #[test]
    fn test_mul_by_zero() {
        let x = Qse::from_unknown_variable("X");
        let y = Qse::from_unknown_variable("Y");
        let a = Qse::from_known_symbol("A", RangeConstraint::default());
        let zero = Qse::from(GoldilocksField::from(0));
        let t: Qse = x * y + a;
        assert_eq!(t.to_string(), "(X) * (Y) + A");
        assert_eq!((t.clone() * zero).to_string(), "0");
    }

    #[test]
    fn test_apply_update() {
        let x = Qse::from_unknown_variable("X");
        let y = Qse::from_unknown_variable("Y");
        let a = Qse::from_known_symbol("A", RangeConstraint::default());
        let b = Qse::from_known_symbol("B", RangeConstraint::default());
        let mut t: Qse = (x * y + a) * b;
        assert_eq!(t.to_string(), "(B * X) * (Y) + (A * B)");
        t.apply_update(&VariableUpdate {
            variable: "B",
            known: true,
            range_constraint: RangeConstraint::from_value(7.into()),
        });
        assert!(t.is_quadratic());
        assert_eq!(t.to_string(), "(7 * X) * (Y) + (A * 7)");
        t.apply_update(&VariableUpdate {
            variable: "X",
            known: true,
            range_constraint: RangeConstraint::from_range(1.into(), 2.into()),
        });
        assert!(!t.is_quadratic());
        assert_eq!(t.to_string(), "(X * 7) * Y + (A * 7)");
        t.apply_update(&VariableUpdate {
            variable: "Y",
            known: true,
            range_constraint: RangeConstraint::from_value(3.into()),
        });
        assert!(t.try_to_known().is_some());
        assert_eq!(t.to_string(), "((A * 7) + (3 * (X * 7)))");
    }

    #[test]
    fn test_apply_update_inner_zero() {
        let x = Qse::from_unknown_variable("X");
        let y = Qse::from_unknown_variable("Y");
        let a = Qse::from_known_symbol("A", RangeConstraint::default());
        let b = Qse::from_known_symbol("B", RangeConstraint::default());
        let mut t: Qse = (x * a + y) * b;
        assert_eq!(t.to_string(), "(A * B) * X + B * Y");
        t.apply_update(&VariableUpdate {
            variable: "B",
            known: true,
            range_constraint: RangeConstraint::from_value(7.into()),
        });
        assert_eq!(t.to_string(), "(A * 7) * X + 7 * Y");
        t.apply_update(&VariableUpdate {
            variable: "A",
            known: true,
            range_constraint: RangeConstraint::from_value(0.into()),
        });
        assert_eq!(t.to_string(), "7 * Y");
    }

    struct NoRangeConstraints;
    impl RangeConstraintProvider<GoldilocksField, &'static str> for NoRangeConstraints {
        fn get(&self, _var: &&'static str) -> RangeConstraint<GoldilocksField> {
            RangeConstraint::default()
        }
    }

    impl RangeConstraintProvider<GoldilocksField, &'static str>
        for HashMap<&'static str, RangeConstraint<GoldilocksField>>
    {
        fn get(&self, var: &&'static str) -> RangeConstraint<GoldilocksField> {
            self.get(var).cloned().unwrap_or_default()
        }
    }

    #[test]
    fn unsolvable() {
        let r = Qse::from(GoldilocksField::from(10)).solve(&NoRangeConstraints);
        assert!(r.is_err());
    }

    #[test]
    fn unsolvable_with_vars() {
        let x = &Qse::from_known_symbol("X", Default::default());
        let y = &Qse::from_known_symbol("Y", Default::default());
        let mut constr = x + y - GoldilocksField::from(10).into();
        // We cannot solve it, but we can also not learn anything new from it.
        let result = constr.solve(&NoRangeConstraints).unwrap();
        assert!(result.complete && result.effects.is_empty());
        // But if we know the values, we can be sure there is a conflict.
        assert!(Qse::from(GoldilocksField::from(10))
            .solve(&NoRangeConstraints)
            .is_err());

        // The same with range constraints that disallow zero.
        constr.apply_update(&VariableUpdate {
            variable: "X",
            known: true,
            range_constraint: RangeConstraint::from_value(5.into()),
        });
        constr.apply_update(&VariableUpdate {
            variable: "Y",
            known: true,
            range_constraint: RangeConstraint::from_range(100.into(), 102.into()),
        });
        assert!(Qse::from(GoldilocksField::from(10))
            .solve(&NoRangeConstraints)
            .is_err());
    }

    #[test]
    fn solvable_without_vars() {
        let constr = Qse::from(GoldilocksField::from(0));
        let result = constr.solve(&NoRangeConstraints).unwrap();
        assert!(result.complete && result.effects.is_empty());
    }

    #[test]
    fn solve_simple_eq() {
        let y = Qse::from_known_symbol("y", Default::default());
        let x = Qse::from_unknown_variable("X");
        // 2 * X + 7 * y - 10 = 0
        let two = Qse::from(GoldilocksField::from(2));
        let seven = Qse::from(GoldilocksField::from(7));
        let ten = Qse::from(GoldilocksField::from(10));
        let constr = two * x + seven * y - ten;
        let result = constr.solve(&NoRangeConstraints).unwrap();
        assert!(result.complete);
        assert_eq!(result.effects.len(), 1);
        let Effect::Assignment(var, expr) = &result.effects[0] else {
            panic!("Expected assignment");
        };
        assert_eq!(var.to_string(), "X");
        assert_eq!(expr.to_string(), "(((7 * y) + -10) / -2)");
    }

    #[test]
    fn solve_div_by_range_constrained_var() {
        let y = Qse::from_known_symbol("y", Default::default());
        let z = Qse::from_known_symbol("z", Default::default());
        let x = Qse::from_unknown_variable("X");
        // z * X + 7 * y - 10 = 0
        let seven = Qse::from(GoldilocksField::from(7));
        let ten = Qse::from(GoldilocksField::from(10));
        let mut constr = z * x + seven * y - ten.clone();
        // If we do not range-constrain z, we cannot solve since we don't know if it might be zero.
        let result = constr.solve(&NoRangeConstraints).unwrap();
        assert!(!result.complete && result.effects.is_empty());
        let z_rc = RangeConstraint::from_range(1.into(), 2.into());
        let range_constraints: HashMap<&'static str, RangeConstraint<GoldilocksField>> =
            HashMap::from([("z", z_rc.clone())]);
        // Just solving without applying the update to the known symbolic expressions
        // does not help either. Note that the argument `&range_constraints` to
        // `solve()` is only used for unknown variables and not for known variables.
        // For the latter to take effect, we need to call `apply_update`.
        let result = constr.solve(&range_constraints).unwrap();
        assert!(!result.complete && result.effects.is_empty());
        constr.apply_update(&VariableUpdate {
            variable: "z",
            known: true,
            range_constraint: z_rc.clone(),
        });
        // Now it should work.
        let result = constr.solve(&range_constraints).unwrap();
        assert!(result.complete);
        let effects = result.effects;
        let Effect::Assignment(var, expr) = &effects[0] else {
            panic!("Expected assignment");
        };
        assert_eq!(var.to_string(), "X");
        assert_eq!(expr.to_string(), "(((7 * y) + -10) / -z)");
    }

    #[test]
    fn solve_bit_decomposition() {
        let rc = RangeConstraint::from_mask(0xffu32);
        // First try without range constrain on a, but on b and c.
        let a = Qse::from_unknown_variable("a");
        let b = Qse::from_unknown_variable("b");
        let c = Qse::from_unknown_variable("c");
        let z = Qse::from_known_symbol("Z", Default::default());
        // a * 0x100 - b * 0x10000 + c * 0x1000000 + 10 + Z = 0
        let ten = Qse::from(GoldilocksField::from(10));
        let mut constr: Qse = a * Qse::from(GoldilocksField::from(0x100))
            - b * Qse::from(GoldilocksField::from(0x10000))
            + c * Qse::from(GoldilocksField::from(0x1000000))
            + ten.clone()
            + z.clone();
        // Without range constraints on a, this is not solvable.
        let mut range_constraints = HashMap::from([("b", rc.clone()), ("c", rc.clone())]);
        constr.apply_update(&VariableUpdate {
            variable: "b",
            known: false,
            range_constraint: rc.clone(),
        });
        constr.apply_update(&VariableUpdate {
            variable: "c",
            known: false,
            range_constraint: rc.clone(),
        });
        let result = constr.solve(&range_constraints).unwrap();
        assert!(!result.complete && result.effects.is_empty());
        // Now add the range constraint on a, it should be solvable.
        range_constraints.insert("a", rc.clone());
        constr.apply_update(&VariableUpdate {
            variable: "a",
            known: false,
            range_constraint: rc.clone(),
        });
        let result = constr.solve(&range_constraints).unwrap();
        assert!(result.complete);

        let [effect] = &result.effects[..] else {
            panic!();
        };
        let Effect::BitDecomposition(BitDecomposition { value, components }) = effect else {
            panic!();
        };
        assert_eq!(format!("{value}"), "(10 + Z)");
        let formatted = components
            .iter()
            .map(|c| {
                format!(
                    "{} = (({value} & 0x{:0x}) >> {}){};\n",
                    c.variable,
                    c.bit_mask,
                    c.exponent,
                    if c.is_negative { " [negative]" } else { "" }
                )
            })
            .join("");

        assert_eq!(
            formatted,
            "\
a = (((10 + Z) & 0xff00) >> 8) [negative];
b = (((10 + Z) & 0xff0000) >> 16);
c = (((10 + Z) & 0xff000000) >> 24) [negative];
"
        );
    }

    #[test]
    fn solve_constraint_transfer() {
        let rc = RangeConstraint::from_mask(0xffu32);
        let a = Qse::from_unknown_variable("a");
        let b = Qse::from_unknown_variable("b");
        let c = Qse::from_unknown_variable("c");
        let z = Qse::from_unknown_variable("Z");
        let range_constraints =
            HashMap::from([("a", rc.clone()), ("b", rc.clone()), ("c", rc.clone())]);
        // a * 0x100 + b * 0x10000 + c * 0x1000000 + 10 - Z = 0
        let ten = Qse::from(GoldilocksField::from(10));
        let constr = a * Qse::from(GoldilocksField::from(0x100))
            + b * Qse::from(GoldilocksField::from(0x10000))
            + c * Qse::from(GoldilocksField::from(0x1000000))
            + ten.clone()
            - z.clone();
        let result = constr.solve(&range_constraints).unwrap();
        assert!(!result.complete);
        let effects = result
            .effects
            .into_iter()
            .map(|effect| match effect {
                Effect::RangeConstraint(v, rc) => format!("{v}: {rc};\n"),
                _ => panic!(),
            })
            .format("")
            .to_string();
        // It appears twice because we solve the positive and the negated equation.
        // Note that the negated version has a different bit mask.
        assert_eq!(
            effects,
            "Z: [10, 4294967050] & 0xffffff0a;
Z: [10, 4294967050] & 0xffffffff;
"
        );
    }
}
