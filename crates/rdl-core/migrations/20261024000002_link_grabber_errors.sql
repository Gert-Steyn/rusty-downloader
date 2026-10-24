-- LinkGrabber: keep the probe's error message, and look links up by URL for de-duplication.
ALTER TABLE link_grabber ADD COLUMN error TEXT;

CREATE INDEX link_grabber_url ON link_grabber (source_url);
