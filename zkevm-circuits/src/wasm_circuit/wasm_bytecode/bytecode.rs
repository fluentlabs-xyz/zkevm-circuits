use halo2_proofs::circuit::Value;
use eth_types::Field;

///
#[derive(Clone, Debug)]
pub struct WasmBytecode {
    /// Raw bytes
    pub bytes: Vec<u8>,
}

impl WasmBytecode {
    /// Construct from bytecode bytes
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Assignments for bytecode table
    pub fn table_assignments<F: Field>(&self) -> Vec<[Value<F>; 2]> {
        let n = 1 + self.bytes.len();
        let mut rows = Vec::with_capacity(n);

        for (idx, byte) in self.bytes.iter().enumerate() {
            let idx_val = Value::known(F::from(idx as u64));
            let mut byte_val = Value::known(F::from(*byte as u64));
            rows.push([
                idx_val,
                byte_val,
            ])
        }
        rows
    }

    /// get byte value
    pub fn get(&self, idx: usize) -> u8 {
        self.bytes[idx]
    }
}

impl From<&eth_types::bytecode::Bytecode> for WasmBytecode {
    fn from(b: &eth_types::bytecode::Bytecode) -> Self {
        WasmBytecode::new(b.to_vec())
    }
}