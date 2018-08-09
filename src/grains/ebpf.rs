use grains::events::{EventCallback, EventHandler};
use grains::perfhandler::PerfHandler;
use grains::sockethandler::SocketHandler;
use grains::BackendHandler;

use redbpf::cpus;
use redbpf::{Module, PerfMap, Result};

use lazy_socket::raw::Socket;

use std::os::unix::io::FromRawFd;

pub struct Grain<T> {
    module: Module,
    native: T,
}

pub trait EBPFGrain<'code> {
    fn code() -> &'code [u8];
    fn get_handler(&self, id: &str) -> EventCallback;
    fn loaded(&mut self, _module: &mut Module) {}
    fn attached(&mut self, _backends: &[BackendHandler]) {}

    fn load(mut self) -> Result<Grain<Self>>
    where
        Self: Sized,
    {
        let mut module = Module::parse(Self::code())?;
        for prog in module.programs.iter_mut() {
            prog.load(module.version, module.license.clone()).unwrap();
        }

        self.loaded(&mut module);
        Ok(Grain {
            module,
            native: self,
        })
    }
}

impl<'code, 'module, T> Grain<T>
where
    T: EBPFGrain<'code>,
{
    pub fn attach_kprobes(&mut self, backends: &[BackendHandler]) -> Vec<Box<dyn EventHandler>> {
        use redbpf::ProgramKind::*;
        for prog in self
            .module
            .programs
            .iter_mut()
            .filter(|p| p.kind == Kprobe || p.kind == Kretprobe)
        {
            println!("Program: {}, {:?}", prog.name, prog.kind);
            prog.attach_probe().unwrap();
        }

        self.native.attached(backends);
        self.bind_perf(backends)
    }

    pub fn attach_xdps(
        &mut self,
        iface: &str,
        backends: &[BackendHandler],
    ) -> Vec<Box<dyn EventHandler>> {
        use redbpf::ProgramKind::*;
        for prog in self.module.programs.iter_mut().filter(|p| p.kind == XDP) {
            println!("Program: {}, {:?}", prog.name, prog.kind);
            prog.attach_xdp(iface).unwrap();
        }

        self.native.attached(backends);
        self.bind_perf(backends)
    }

    fn bind_perf(&mut self, backends: &[BackendHandler]) -> Vec<Box<dyn EventHandler>> {
        let online_cpus = cpus::get_online().unwrap();
        let mut output: Vec<Box<dyn EventHandler>> = vec![];
        for ref mut m in self.module.maps.iter_mut().filter(|m| m.kind == 4) {
            for cpuid in online_cpus.iter() {
                let pm = PerfMap::bind(m, -1, *cpuid, 16, -1, 0).unwrap();
                output.push(Box::new(PerfHandler {
                    name: m.name.clone(),
                    perfmap: pm,
                    callback: self.native.get_handler(m.name.as_str()),
                    backends: backends.to_vec(),
                }));
            }
        }

        output
    }

    pub fn attach_socketfilters(
        mut self,
        iface: &str,
        backends: &[BackendHandler],
    ) -> Vec<Box<dyn EventHandler>> {
        use redbpf::ProgramKind::*;
        let socket_fds = self
            .module
            .programs
            .iter_mut()
            .filter(|p| p.kind == SocketFilter)
            .map(|prog| {
                println!("Program: {}, {:?}", prog.name, prog.kind);
                prog.attach_socketfilter(iface).unwrap()
            }).collect::<Vec<_>>();

        // we need to get out of mutable borrow land to continue.
        // this is because we cannot simultaneously borrow the `native` as
        // immutable and `programs ` as mutable
        // Therefore it is needed to refilter, but after that ordering should be
        // the same
        let handlers = self
            .module
            .programs
            .iter()
            .filter(|p| p.kind == SocketFilter)
            .zip(&socket_fds)
            .map(|(prog, fd)| {
                Box::new(SocketHandler {
                    socket: unsafe { Socket::from_raw_fd(*fd) },
                    backends: backends.to_vec(),
                    callback: self.native.get_handler(prog.name.as_str()),
                }) as Box<dyn EventHandler>
            }).collect();

        self.native.attached(backends);
        handlers
    }
}
