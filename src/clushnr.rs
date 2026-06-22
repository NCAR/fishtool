pub mod clushnr {
    use std::process::Command;
    pub fn get_nodes(noderange:String) -> Vec<String> {
        let mut nodes = Vec::new();
        let outp = Command::new("cluset")
                                       .arg("-e")
                                       .arg(noderange)
                                       .output().expect("Error running cluset");
        let nodestring = String::from_utf8(outp.stdout).unwrap();
        for n in nodestring.split_ascii_whitespace() {
            nodes.push(n.to_string());
        }
        return nodes
    }
}