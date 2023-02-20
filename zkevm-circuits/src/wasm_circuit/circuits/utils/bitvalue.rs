use std::marker::PhantomData;

use super::Context;
use crate::wasm_circuit::{circuits::rtable::RangeTableConfig};
use halo2_proofs::{
    arithmetic::FieldExt,
    plonk::{Advice, Column, ConstraintSystem, Error, Expression, VirtualCells},
};
use halo2_proofs::circuit::Value;
use eth_types::Field;
use crate::{constant, curr};

#[derive(Clone)]
pub struct BitValueConfig<F: Field> {
    pub bits_le: [Column<Advice>; 16],
    pub value: Column<Advice>,
    _mark: PhantomData<F>,
}

impl<F: Field> BitValueConfig<F> {
    pub fn configure(
        meta: &mut ConstraintSystem<F>,
        cols: &mut impl Iterator<Item = Column<Advice>>,
        rtable: &RangeTableConfig<F>,
        enable: impl Fn(&mut VirtualCells<'_, F>) -> Expression<F>,
    ) -> Self {
        let bits_le = [(); 16].map(|_| cols.next().unwrap());
        let value = cols.next().unwrap();

        for i in 0..16 {
            rtable.configure_in_u4_range(meta, "bits repr", |meta| {
                curr!(meta, bits_le[i]) * enable(meta)
            });
        }

        meta.create_gate("bits le sum", |meta| {
            let mut acc = curr!(meta, bits_le[0].clone());
            let mut base = F::one();
            for i in 1..16usize {
                base = base * F::from(16u64);
                acc = acc + constant!(base) * curr!(meta, bits_le[i].clone());
            }
            vec![(acc - curr!(meta, value.clone())) * enable(meta)]
        });

        Self {
            bits_le,
            value,
            _mark: PhantomData,
        }
    }

    pub fn assign(&self, ctx: &mut Context<F>, value: u64) -> Result<(), Error> {
        let mut v = value;
        ctx.region
            .assign_advice(|| "value", self.value, ctx.offset, || Value::known(F::from(value)))?;

        for i in 0..16 {
            ctx.region.assign_advice(
                || "tvalue vtype",
                self.bits_le[i],
                ctx.offset,
                || Value::<F>::known((v & 15).into()),
            )?;

            v >>= 4;
        }

        Ok(())
    }
}