use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use halo2_proofs::{
    circuit::{Region, Value},
    plonk::{Advice, Column, ConstraintSystem, Fixed},
    poly::Rotation,
};
use itertools::Itertools;
use log::debug;

use eth_types::Field;
use gadgets::{
    binary_number::BinaryNumberChip,
    util::{and, not, or, Expr},
};

use crate::{
    evm_circuit::util::constraint_builder::{BaseConstraintBuilder, ConstrainBuilderCommon},
    wasm_circuit::{
        bytecode::{bytecode::WasmBytecode, bytecode_table::WasmBytecodeTable},
        common::{
            configure_constraints_for_q_first_and_q_last, configure_transition_check,
            WasmAssignAwareChip, WasmCountPrefixedItemsAwareChip, WasmErrorAwareChip,
            WasmFuncCountAwareChip, WasmMarkupLeb128SectionAwareChip, WasmSharedStateAwareChip,
        },
        consts::WASM_BLOCK_END,
        error::{remap_error_to_assign_at, remap_error_to_invalid_enum_value_at, Error},
        leb128::circuit::LEB128Chip,
        sections::{consts::LebParams, global::body::types::AssignType},
        tables::dynamic_indexes::{
            circuit::DynamicIndexesChip,
            types::{LookupArgsParams, Tag},
        },
        types::{
            AssignDeltaType, AssignValueType, NewWbOffsetType, NumType, NumericInstruction,
            SharedState, NUM_TYPE_VALUES,
        },
    },
};

#[derive(Debug, Clone)]
pub struct WasmGlobalSectionBodyConfig<F: Field> {
    pub q_enable: Column<Fixed>,
    pub q_first: Column<Fixed>,
    pub q_last: Column<Fixed>,
    pub is_items_count: Column<Fixed>,
    pub is_global_type: Column<Fixed>,
    pub is_global_type_ctx: Column<Fixed>,
    pub is_mut_prop: Column<Fixed>,
    pub is_init_opcode: Column<Fixed>,
    pub is_init_val: Column<Fixed>,
    pub is_expr_delimiter: Column<Fixed>,

    pub global_type: Column<Advice>,

    pub leb128_chip: Rc<LEB128Chip<F>>,
    pub dynamic_indexes_chip: Rc<DynamicIndexesChip<F>>,
    pub global_type_chip: Rc<BinaryNumberChip<F, NumType, 8>>,

    func_count: Column<Advice>,
    body_item_rev_count: Column<Advice>,

    error_code: Column<Advice>,

    shared_state: Rc<RefCell<SharedState>>,

    _marker: PhantomData<F>,
}

impl<'a, F: Field> WasmGlobalSectionBodyConfig<F> {}

#[derive(Debug, Clone)]
pub struct WasmGlobalSectionBodyChip<F: Field> {
    pub config: WasmGlobalSectionBodyConfig<F>,
    _marker: PhantomData<F>,
}

impl<F: Field> WasmMarkupLeb128SectionAwareChip<F> for WasmGlobalSectionBodyChip<F> {}

impl<F: Field> WasmCountPrefixedItemsAwareChip<F> for WasmGlobalSectionBodyChip<F> {}

impl<F: Field> WasmErrorAwareChip<F> for WasmGlobalSectionBodyChip<F> {
    fn error_code_col(&self) -> Column<Advice> {
        self.config.error_code
    }
}

impl<F: Field> WasmSharedStateAwareChip<F> for WasmGlobalSectionBodyChip<F> {
    fn shared_state(&self) -> Rc<RefCell<SharedState>> {
        self.config.shared_state.clone()
    }
}

impl<F: Field> WasmFuncCountAwareChip<F> for WasmGlobalSectionBodyChip<F> {
    fn func_count_col(&self) -> Column<Advice> {
        self.config.func_count
    }
}

impl<F: Field> WasmAssignAwareChip<F> for WasmGlobalSectionBodyChip<F> {
    type AssignType = AssignType;

    fn assign_internal(
        &self,
        region: &mut Region<F>,
        wb: &WasmBytecode,
        wb_offset: usize,
        assign_delta: AssignDeltaType,
        assign_types: &[Self::AssignType],
        assign_value: AssignValueType,
        leb_params: Option<LebParams>,
    ) -> Result<(), Error> {
        let q_enable = true;
        let assign_offset = wb_offset + assign_delta;
        debug!(
            "assign at {} q_enable {} assign_types {:?} assign_values {} byte_val {:x?}",
            assign_offset, q_enable, assign_types, assign_value, wb.bytes[wb_offset],
        );
        region
            .assign_fixed(
                || format!("assign 'q_enable' val {} at {}", q_enable, assign_offset),
                self.config.q_enable,
                assign_offset,
                || Value::known(F::from(q_enable as u64)),
            )
            .map_err(remap_error_to_assign_at(assign_offset))?;
        self.assign_func_count(region, assign_offset)?;

        for assign_type in assign_types {
            if [AssignType::IsItemsCount, AssignType::IsInitVal].contains(&assign_type) {
                let p = leb_params.unwrap();
                self.config
                    .leb128_chip
                    .assign(region, assign_offset, q_enable, p)?;
            }
            match assign_type {
                AssignType::QFirst => {
                    region
                        .assign_fixed(
                            || {
                                format!(
                                    "assign 'q_first' val {} at {}",
                                    assign_value, assign_offset
                                )
                            },
                            self.config.q_first,
                            assign_offset,
                            || Value::known(F::from(assign_value)),
                        )
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                }
                AssignType::QLast => {
                    region
                        .assign_fixed(
                            || format!("assign 'q_last' val {} at {}", assign_value, assign_offset),
                            self.config.q_last,
                            assign_offset,
                            || Value::known(F::from(assign_value)),
                        )
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                }
                AssignType::IsItemsCount => {
                    region
                        .assign_fixed(
                            || {
                                format!(
                                    "assign 'is_items_count' val {} at {}",
                                    assign_value, assign_offset
                                )
                            },
                            self.config.is_items_count,
                            assign_offset,
                            || Value::known(F::from(assign_value)),
                        )
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                }
                AssignType::IsGlobalType => {
                    region
                        .assign_fixed(
                            || {
                                format!(
                                    "assign 'is_global_type' val {} at {}",
                                    assign_value, assign_offset
                                )
                            },
                            self.config.is_global_type,
                            assign_offset,
                            || Value::known(F::from(assign_value)),
                        )
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                }
                AssignType::IsMutProp => {
                    region
                        .assign_fixed(
                            || {
                                format!(
                                    "assign 'is_mut_prop' val {} at {}",
                                    assign_value, assign_offset
                                )
                            },
                            self.config.is_mut_prop,
                            assign_offset,
                            || Value::known(F::from(assign_value)),
                        )
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                }
                AssignType::IsInitOpcode => {
                    region
                        .assign_fixed(
                            || {
                                format!(
                                    "assign 'is_init_opcode' val {} at {}",
                                    assign_value, assign_offset
                                )
                            },
                            self.config.is_init_opcode,
                            assign_offset,
                            || Value::known(F::from(assign_value)),
                        )
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                }
                AssignType::IsInitVal => {
                    region
                        .assign_fixed(
                            || {
                                format!(
                                    "assign 'is_init_val' val {} at {}",
                                    assign_value, assign_offset
                                )
                            },
                            self.config.is_init_val,
                            assign_offset,
                            || Value::known(F::from(assign_value)),
                        )
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                }
                AssignType::IsExprDelimiter => {
                    region
                        .assign_fixed(
                            || {
                                format!(
                                    "assign 'is_expr_delimiter' val {} at {}",
                                    assign_value, assign_offset
                                )
                            },
                            self.config.is_expr_delimiter,
                            assign_offset,
                            || Value::known(F::from(assign_value)),
                        )
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                }
                AssignType::GlobalType => {
                    region
                        .assign_advice(
                            || {
                                format!(
                                    "assign 'global_type' val {} at {}",
                                    assign_value, assign_offset
                                )
                            },
                            self.config.global_type,
                            assign_offset,
                            || Value::known(F::from(assign_value)),
                        )
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                    let global_type: NumType = (assign_value as u8)
                        .try_into()
                        .map_err(remap_error_to_invalid_enum_value_at(assign_offset))?;
                    self.config
                        .global_type_chip
                        .assign(region, assign_offset, &global_type)
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                }
                AssignType::IsGlobalTypeCtx => {
                    region
                        .assign_fixed(
                            || {
                                format!(
                                    "assign 'is_global_type_ctx' val {} at {}",
                                    assign_value, assign_offset
                                )
                            },
                            self.config.is_global_type_ctx,
                            assign_offset,
                            || Value::known(F::from(assign_value)),
                        )
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                }
                AssignType::BodyItemRevCount => {
                    region
                        .assign_advice(
                            || {
                                format!(
                                    "assign 'body_item_rev_count' val {} at {}",
                                    assign_value, assign_offset
                                )
                            },
                            self.config.body_item_rev_count,
                            assign_offset,
                            || Value::known(F::from(assign_value)),
                        )
                        .map_err(remap_error_to_assign_at(assign_offset))?;
                }
                AssignType::ErrorCode => {
                    self.assign_error_code(region, assign_offset, None)?;
                }
            }
        }
        Ok(())
    }
}

impl<F: Field> WasmGlobalSectionBodyChip<F> {
    pub fn construct(config: WasmGlobalSectionBodyConfig<F>) -> Self {
        let instance = Self {
            config,
            _marker: PhantomData,
        };
        instance
    }

    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        wb_table: Rc<WasmBytecodeTable>,
        leb128_chip: Rc<LEB128Chip<F>>,
        dynamic_indexes_chip: Rc<DynamicIndexesChip<F>>,
        func_count: Column<Advice>,
        shared_state: Rc<RefCell<SharedState>>,
        body_item_rev_count: Column<Advice>,
        error_code: Column<Advice>,
        bytecode_number: Column<Advice>,
    ) -> WasmGlobalSectionBodyConfig<F> {
        let q_enable = cs.fixed_column();
        let q_first = cs.fixed_column();
        let q_last = cs.fixed_column();
        let is_items_count = cs.fixed_column();
        let is_global_type = cs.fixed_column();
        let is_global_type_ctx = cs.fixed_column();
        let is_mut_prop = cs.fixed_column();
        let is_init_opcode = cs.fixed_column();
        let is_init_val = cs.fixed_column();
        let is_expr_delimiter = cs.fixed_column();

        let global_type = cs.advice_column();
        let config = BinaryNumberChip::configure(cs, is_global_type_ctx, Some(global_type.into()));
        let global_type_chip = Rc::new(BinaryNumberChip::construct(config));

        dynamic_indexes_chip.lookup_args(
            "global section has valid setup for mem indexes",
            cs,
            |vc| {
                let cond = vc.query_fixed(is_items_count, Rotation::cur());
                let cond = cond
                    * Self::get_selector_expr_enriched_with_error_processing(
                        vc,
                        q_enable,
                        &shared_state.borrow(),
                        error_code,
                    );
                LookupArgsParams {
                    cond,
                    bytecode_number: vc.query_advice(bytecode_number, Rotation::cur()),
                    index: vc.query_advice(leb128_chip.config.sn, Rotation::cur()),
                    tag: Tag::GlobalIndex.expr(),
                    is_terminator: true.expr(),
                }
            },
        );

        Self::configure_count_prefixed_items_checks(
            cs,
            leb128_chip.as_ref(),
            body_item_rev_count,
            |vc| vc.query_fixed(is_items_count, Rotation::cur()),
            |vc| {
                let q_enable_expr = Self::get_selector_expr_enriched_with_error_processing(
                    vc,
                    q_enable,
                    &shared_state.borrow(),
                    error_code,
                );
                let is_items_count_expr = vc.query_fixed(is_items_count, Rotation::cur());

                and::expr([q_enable_expr, not::expr(is_items_count_expr)])
            },
            |vc| vc.query_fixed(is_global_type, Rotation::cur()),
            |vc| vc.query_fixed(q_last, Rotation::cur()),
        );

        cs.create_gate("WasmGlobalSectionBody gate", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let q_enable_expr = Self::get_selector_expr_enriched_with_error_processing(vc, q_enable, &shared_state.borrow(), error_code);
            // let q_first_expr = vc.query_fixed(q_first, Rotation::cur());
            let q_last_expr = vc.query_fixed(q_last, Rotation::cur());
            let not_q_last_expr = not::expr(q_last_expr.clone());
            let is_items_count_expr = vc.query_fixed(is_items_count, Rotation::cur());
            let is_global_type_expr = vc.query_fixed(is_global_type, Rotation::cur());
            let is_global_type_ctx_expr = vc.query_fixed(is_global_type_ctx, Rotation::cur());
            let is_mut_prop_expr = vc.query_fixed(is_mut_prop, Rotation::cur());
            let is_init_opcode_expr = vc.query_fixed(is_init_opcode, Rotation::cur());
            let is_init_val_expr = vc.query_fixed(is_init_val, Rotation::cur());
            let is_expr_delimiter_expr = vc.query_fixed(is_expr_delimiter, Rotation::cur());

            let byte_val_expr = vc.query_advice(wb_table.value, Rotation::cur());

            let global_type_expr = vc.query_advice(global_type, Rotation::cur());

            let leb128_is_last_byte_expr = vc.query_fixed(leb128_chip.config.is_last_byte, Rotation::cur());

            cb.require_boolean("q_enable is boolean", q_enable_expr.clone());
            cb.require_boolean("is_items_count is boolean", is_items_count_expr.clone());
            cb.require_boolean("is_global_type is boolean", is_global_type_expr.clone());
            cb.require_boolean("is_mut_prop is boolean", is_mut_prop_expr.clone());
            cb.require_boolean("is_init_opcode is boolean", is_init_opcode_expr.clone());
            cb.require_boolean("is_init_val is boolean", is_init_val_expr.clone());
            cb.require_boolean("is_expr_delimiter is boolean", is_expr_delimiter_expr.clone());

            configure_constraints_for_q_first_and_q_last(
                &mut cb,
                vc,
                &q_enable,
                &q_first,
                &[is_items_count],
                &q_last,
                &[is_expr_delimiter],
            );

            cb.require_equal(
                "exactly one mark flag active at the same time",
                is_items_count_expr.clone()
                    + is_global_type_expr.clone()
                    + is_mut_prop_expr.clone()
                    + is_init_opcode_expr.clone()
                    + is_init_val_expr.clone()
                    + is_expr_delimiter_expr.clone()
                ,
                1.expr(),
            );

            cb.condition(
                is_global_type_expr.clone(),
                |cb| {
                    let global_type_expr = vc.query_advice(global_type, Rotation::cur());
                    cb.require_equal(
                        "is_global_type => global_type=byte_val",
                        global_type_expr,
                        byte_val_expr.clone(),
                    );
                }
            );
            cb.require_equal(
                "is_global_type_ctx active on a specific flags only",
                is_global_type_expr.clone()
                    + is_mut_prop_expr.clone()
                    + is_init_opcode_expr.clone()
                    + is_init_val_expr.clone()
                ,
                is_global_type_ctx_expr.clone(),
            );
            cb.condition(
                is_global_type_ctx_expr.clone(),
                |cb| {
                    let is_global_type_ctx_prev_expr = vc.query_fixed(is_global_type_ctx, Rotation::prev());
                    let global_type_prev_expr = vc.query_advice(global_type, Rotation::prev());
                    cb.require_zero(
                        "is_global_type_ctx && prev.is_global_type_ctx => ",
                        is_global_type_ctx_prev_expr.clone() * (global_type_prev_expr.clone() - global_type_expr.clone()),
                    );
                }
            );

            cb.condition(
                or::expr([
                    is_items_count_expr.clone(),
                    is_init_val_expr.clone(),
                ]),
                |cb| {
                    cb.require_equal(
                        "is_items_count || is_init_val -> leb128",
                        vc.query_fixed(leb128_chip.config.q_enable, Rotation::cur()),
                        1.expr(),
                    )
                }
            );

            // is_items_count+ -> item+(is_global_type{1} -> is_mut_prop{1} -> is_init_opcode{1} -> is_init_val+ -> is_expr_delimiter{1})
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_items_count+ -> item+(is_global_type{1} ...",
                and::expr([
                    not_q_last_expr.clone(),
                    is_items_count_expr.clone(),
                ]),
                true,
                &[is_items_count, is_global_type],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_items_count+ -> item+(is_global_type{1} ...",
                and::expr([
                    not_q_last_expr.clone(),
                    leb128_is_last_byte_expr.clone(),
                    is_items_count_expr.clone(),
                ]),
                true,
                &[is_global_type],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_global_type{1} -> is_mut_prop{1}",
                and::expr([
                    not_q_last_expr.clone(),
                    is_global_type_expr.clone(),
                ]),
                true,
                &[is_mut_prop, ],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mut_prop{1} -> is_init_opcode{1}",
                and::expr([
                    not_q_last_expr.clone(),
                    is_mut_prop_expr.clone(),
                ]),
                true,
                &[is_init_opcode, ],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_init_opcode{1} -> is_init_val+",
                and::expr([
                    not_q_last_expr.clone(),
                    is_init_opcode_expr.clone(),
                ]),
                true,
                &[is_init_val, ],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_init_val+ -> is_expr_delimiter{1}",
                and::expr([
                    not_q_last_expr.clone(),
                    is_init_val_expr.clone(),
                ]),
                true,
                &[is_init_val, is_expr_delimiter],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_init_val+ -> is_expr_delimiter{1}",
                and::expr([
                    not_q_last_expr.clone(),
                    leb128_is_last_byte_expr.clone(),
                    is_init_val_expr.clone(),
                ]),
                true,
                &[is_expr_delimiter],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_expr_delimiter{1}",
                and::expr([
                    not_q_last_expr.clone(),
                    is_expr_delimiter_expr.clone(),
                ]),
                true,
                &[is_global_type],
            );

            cb.condition(
                is_global_type_expr.clone(),
                |cb| {
                    cb.require_in_set(
                        "is_global_type has eligible byte value",
                        byte_val_expr.clone(),
                        NUM_TYPE_VALUES.iter().map(|&v| v.expr()).collect_vec(),
                    )
                }
            );

            cb.condition(
                is_mut_prop_expr.clone(),
                |cb| {
                    cb.require_boolean(
                        "is_mut_prop -> bool",
                        byte_val_expr.clone(),
                    )
                }
            );

            cb.condition(
                is_init_opcode_expr.clone(),
                |cb| {
                    cb.require_in_set(
                        "is_init_opcode has eligible byte value",
                        byte_val_expr.clone(),
                        vec![
                            NumericInstruction::I32Const.expr(),
                            NumericInstruction::I64Const.expr(),
                            // add support for float types?
                            // F32Const,
                            // F64Const,
                        ],
                    );
                    let global_type_is_i32_expr = global_type_chip.config.value_equals(NumType::I32, Rotation::cur())(vc);
                    cb.require_zero(
                        "is_init_opcode && global_type_is_i32 => global type corresponds to init opcode",
                        global_type_is_i32_expr * (NumType::I32.expr() - byte_val_expr.clone() - (NumType::I32 as i32 - NumericInstruction::I32Const as i32).expr()),
                    );
                    let global_type_is_i64_expr = global_type_chip.config.value_equals(NumType::I64, Rotation::cur())(vc);
                    cb.require_zero(
                        "is_init_opcode && global_type_is_i64 => global type corresponds to init opcode",
                        global_type_is_i64_expr * (NumType::I64.expr() - byte_val_expr.clone() - (NumType::I64 as i32 - NumericInstruction::I64Const as i32).expr()),
                    );
                }
            );

            cb.condition(
                is_expr_delimiter_expr.clone(),
                |cb| {
                    cb.require_equal(
                        "is_expr_delimiter -> byte value = WASM_BLOCK_END",
                        byte_val_expr.clone(),
                        WASM_BLOCK_END.expr(),
                    )
                }
            );

            cb.gate(q_enable_expr.clone())
        });

        let config = WasmGlobalSectionBodyConfig::<F> {
            _marker: PhantomData,

            q_enable,
            q_first,
            q_last,
            is_items_count,
            is_global_type,
            is_global_type_ctx,
            is_mut_prop,
            is_init_opcode,
            is_init_val,
            is_expr_delimiter,
            global_type,
            leb128_chip,
            dynamic_indexes_chip,
            global_type_chip,
            func_count,
            body_item_rev_count,
            error_code,
            shared_state,
        };

        config
    }

    pub fn assign_auto(
        &self,
        region: &mut Region<F>,
        wb: &WasmBytecode,
        wb_offset: usize,
        assign_delta: AssignDeltaType,
    ) -> Result<NewWbOffsetType, Error> {
        let mut offset = wb_offset;

        let (items_count, items_count_leb_len) = self.markup_leb_section(
            region,
            wb,
            offset,
            assign_delta,
            &[AssignType::IsItemsCount],
        )?;
        let mut body_item_rev_count = items_count;
        for offset in offset..offset + items_count_leb_len {
            self.assign(
                region,
                &wb,
                offset,
                assign_delta,
                &[AssignType::BodyItemRevCount],
                body_item_rev_count,
                None,
            )?;
        }
        let dynamic_indexes_offset = self.config.dynamic_indexes_chip.assign_auto(
            region,
            self.config.shared_state.borrow().dynamic_indexes_offset,
            assign_delta,
            items_count as usize,
            Tag::GlobalIndex,
        )?;
        self.config.shared_state.borrow_mut().dynamic_indexes_offset = dynamic_indexes_offset;
        self.assign(
            region,
            &wb,
            offset,
            assign_delta,
            &[AssignType::QFirst],
            1,
            None,
        )?;
        offset += items_count_leb_len;

        for _item_index in 0..items_count {
            body_item_rev_count -= 1;
            let item_start_offset = offset;

            // is_global_type{1}
            let global_type_val = wb.bytes[offset];
            // let global_type: NumType =
            // global_type_val.try_into().map_err(remap_error_to_invalid_enum_value_at(offset))?;
            let global_type_val = global_type_val as u64;
            self.assign(
                region,
                wb,
                offset,
                assign_delta,
                &[AssignType::IsGlobalType, AssignType::IsGlobalTypeCtx],
                1,
                None,
            )?;
            self.assign(
                region,
                wb,
                offset,
                assign_delta,
                &[AssignType::GlobalType],
                global_type_val,
                None,
            )?;
            offset += 1;

            // is_mut_prop{1}
            self.assign(
                region,
                wb,
                offset,
                assign_delta,
                &[AssignType::IsMutProp, AssignType::IsGlobalTypeCtx],
                1,
                None,
            )?;
            self.assign(
                region,
                wb,
                offset,
                assign_delta,
                &[AssignType::GlobalType],
                global_type_val,
                None,
            )?;
            offset += 1;

            // is_init_opcode{1}
            self.assign(
                region,
                wb,
                offset,
                assign_delta,
                &[AssignType::IsInitOpcode, AssignType::IsGlobalTypeCtx],
                1,
                None,
            )?;
            self.assign(
                region,
                wb,
                offset,
                assign_delta,
                &[AssignType::GlobalType],
                global_type_val,
                None,
            )?;
            offset += 1;

            // is_init_val+
            let (_init_val, init_val_leb_len) = self.markup_leb_section(
                region,
                wb,
                offset,
                assign_delta,
                &[AssignType::IsInitVal, AssignType::IsGlobalTypeCtx],
            )?;
            for offset in offset..offset + init_val_leb_len {
                self.assign(
                    region,
                    wb,
                    offset,
                    assign_delta,
                    &[AssignType::GlobalType],
                    global_type_val,
                    None,
                )?;
            }
            offset += init_val_leb_len;

            // is_expr_delimiter{1}
            self.assign(
                region,
                wb,
                offset,
                assign_delta,
                &[AssignType::IsExprDelimiter],
                1,
                None,
            )?;
            offset += 1;

            for offset in item_start_offset..offset {
                self.assign(
                    region,
                    &wb,
                    offset,
                    assign_delta,
                    &[AssignType::BodyItemRevCount],
                    body_item_rev_count,
                    None,
                )?;
            }
        }

        if offset != wb_offset {
            self.assign(
                region,
                &wb,
                offset - 1,
                assign_delta,
                &[AssignType::QLast],
                1,
                None,
            )?;
        }

        Ok(offset)
    }
}
