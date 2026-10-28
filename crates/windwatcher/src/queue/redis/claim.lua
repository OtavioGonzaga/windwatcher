local now = ARGV[3]
for i=1,64 do
    local a = first_due('pending',now)
    local b = first_due('leased',now)
    if not a and not b then return {} end
    local m = a
    local old = 'pending'
    if b and (not a or b < a) then m = b; old = 'leased' end
    local id = job_id(m)
    local k = hash(id)
    if redis.call('HGET',k,'status') ~= old then error('inconsistent queue index') end
    local attempts = tonumber(redis.call('HGET',k,'attempts'))
    local maximum = tonumber(redis.call('HGET',k,'max_attempts'))
    if not attempts or not maximum or attempts < 0 or maximum < 1 or maximum > 4294967295 then error('invalid queue attempts') end
    if attempts >= maximum then
        if old ~= 'leased' then error('exhausted pending job') end
        exhausted(k,id,now)
    else
        -- Validate metadata before mutation; payload remains opaque.
        member(k,id)
        remove_due(k,id,old)
        move(k,id,old,'leased')
        redis.call('HSET',k,'attempts',tostring(attempts+1),'lease_id',ARGV[1],'lease_owner',ARGV[2],'lease_expires_at',ARGV[4])
        redis.call('ZADD',due('leased'),0,ARGV[4] .. ':' .. id)
        return redis.call('HGETALL',k)
    end
end
return {}
