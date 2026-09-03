use az_proto_build::ProtoCrate;

fn main() {
    az_proto_build::compile(
        ProtoCrate::Session,
        &[
            ProtoCrate::Authoring,
            ProtoCrate::Core,
            ProtoCrate::Project,
            ProtoCrate::Asset,
            ProtoCrate::Runtime,
        ],
    );
}
