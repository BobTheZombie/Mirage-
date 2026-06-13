# Legacy subkernel notes

The historical subkernel name is obsolete for active architecture ownership. Runtime policy belongs to the Supervisor, kernel component startup belongs to Mirage-dispatch-rs, and execution belongs to MTSS.

The current `src/subkernel` code is not blindly deleted because active capability/security paths still reference its types. Its remaining useful pieces should be migrated into precise modules such as `mirage-cap` or Supervisor security policy, after which the subkernel name can be removed from the active boot path.
