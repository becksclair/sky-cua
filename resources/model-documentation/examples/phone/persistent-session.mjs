{
  const { phone } = await import("@heliasar/sky-cua/phone");
  const listing = await phone.list_devices();
  if (listing.devices.length === 0) {
    nodeRepl.write({ available: false, devices: [] });
  } else {
    globalThis.android ??= await phone.connect({ serial: listing.devices[0].serial });
    const shot = await android.screenshot();
    nodeRepl.write({ available: true, device: listing.devices[0], metadata: shot.response });
    await shot.emitImage();
  }
}
