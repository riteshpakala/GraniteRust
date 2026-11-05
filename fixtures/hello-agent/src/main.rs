//! The runner: the package's agents, answered on the wire.

fn main() {
    granite_agents::AgentServer::main(granite_agents::roster![hello_agent::Hello])
}
