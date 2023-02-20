use crate::wasm_circuit::{
    circuits::{config::zkwasm_k, TestCircuit},
};

use halo2_proofs::{arithmetic::FieldExt, dev::MockProver};
use crate::wasm_circuit::specs::Tables;
use std::collections::HashMap;
use halo2_proofs::halo2curves::pasta::Fp;
use halo2_proofs::plonk::Error;
use eth_types::Field;

mod spec;

pub fn test_circuit_noexternal(textual_repr: &str) -> Result<(), ()> {
    panic!("not implemented: {}", textual_repr)
    // let wasm = wabt::wat2wasm(&textual_repr).expect("failed to parse wat");
    //
    // let compiler = WasmInterpreter::new();
    // let compiled_module = compiler
    //     .compile(&wasm, &ImportsBuilder::default(), &HashMap::default())
    //     .unwrap();
    // let execution_result = compiled_module.run(&mut NopExternals, "test")?;
    //
    // run_test_circuit::<Fp>(execution_result.tables, vec![])
}

pub fn run_test_circuit<F: Field>(tables: Tables, public_inputs: Vec<F>) -> Result<(), Error> {
    tables.write_json(None);

    let circuit = TestCircuit::<F>::new(tables);

    let prover = MockProver::run(zkwasm_k(), &circuit, vec![public_inputs])?;
    assert_eq!(prover.verify(), Ok(()));

    Ok(())
}