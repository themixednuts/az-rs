use az_proto_build::ProtoCrate;

fn main() {
    az_proto_build::compile(ProtoCrate::Runtime, &[ProtoCrate::Core]);
}
