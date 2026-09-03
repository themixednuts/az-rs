use az_proto_build::ProtoCrate;

fn main() {
    az_proto_build::compile(
        ProtoCrate::Daemon,
        &[
            ProtoCrate::Authoring,
            ProtoCrate::Core,
            ProtoCrate::Session,
            ProtoCrate::Project,
            ProtoCrate::Asset,
            ProtoCrate::Runtime,
        ],
    );
}
