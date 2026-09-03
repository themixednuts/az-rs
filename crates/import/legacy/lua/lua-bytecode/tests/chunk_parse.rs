use az_lua_bytecode::{
    chunk::{Constant, Proto},
    parse_chunk,
    version::LuaTarget,
};

const SYNTHETIC_METHOD: &[u8] = include_bytes!("fixtures/linear/method_string.luac");

#[test]
fn parses_synthetic_lua_51_chunk() {
    let chunk = parse_chunk(SYNTHETIC_METHOD).expect("synthetic chunk parses");

    assert_eq!(chunk.header.version, LuaTarget::V51);
    assert_eq!(chunk.header.format, 0);
    assert_eq!(chunk.header.instruction_size, 4);
    assert!(!chunk.root.code.is_empty());
    assert!(!chunk.root.constants.is_empty());
    assert!(chunk.root.max_stack > 0);
    assert!(chunk.root.num_params <= chunk.root.max_stack);

    let mut string_constants = 0;
    visit_protos(&chunk.root, &mut |proto| {
        assert!(proto.max_stack > 0);
        assert!(proto.num_params <= proto.max_stack);
        for constant in &proto.constants {
            match constant {
                Constant::Nil | Constant::Boolean(_) | Constant::Number(_) => {}
                Constant::Integer(_) => {
                    panic!("Lua 5.1 chunks should not contain integer constants")
                }
                Constant::Str(bytes) => {
                    string_constants += 1;
                    let _ = bytes.as_slice();
                }
            }
        }
    });
    assert!(string_constants > 0);
}

fn visit_protos(proto: &Proto, visitor: &mut impl FnMut(&Proto)) {
    visitor(proto);
    for nested in &proto.protos {
        visit_protos(nested, visitor);
    }
}
